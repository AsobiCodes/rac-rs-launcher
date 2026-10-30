//! The mod loader card on the Mods page: which OmniLoader is installed next to the game, whether a newer one is out (GitHub
//! AsobiCodes/omniloader releases, checked the same way the launcher checks for game and mod updates), and the buttons "Update loader",
//! "Go back to the previous loader" and "Use the game's own loader". The rules (the ABI the game supports, SHA-256, never while the game
//! runs, `.prev` for the way back) are in `mods/loader_update.rs`; this file is only the page and its background threads.
//! One check per launcher start and per install (game folder + game version), and only when the game is installed, it is version 0.4.2
//! or newer (older ones cannot be asked which interface they serve), script mods are supported and a loader library is next to the exe.
//! A verdict belongs to the exe it was made for: another install shows "Looking for..." until its own check ends, and a check that
//! ends after a newer one began is thrown away. A failed check stays a grey line; nothing here nags or blocks the page, and the local
//! buttons (go back, the game's own loader) never wait for the network.
use super::state::App;
use super::theme;
use crate::gating;
use crate::mods::loader;
use crate::mods::loader_update::{self as lu, Offer, Verdict};
use crate::platform;
use crate::releases::{self, ReleaseSource, Transfer};
use egui::{RichText, Ui};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the last look at the loader releases found.
pub enum Check {
    Idle,
    Checking,
    Done(Box<Verdict>),
    Failed(String),
}

/// A change that runs on a worker thread (install, go back, use the game's own loader).
pub struct Job {
    pub what: &'static str,
    pub transfer: Arc<Transfer>,
}

struct Shared {
    check: Check,
    /// The game exe `check`, `restore_ok` and `installed` were made for (nothing is shown for any other exe).
    for_exe: Option<PathBuf>,
    /// Counts the checks started; a check that ends when a newer one has begun is dropped.
    serial: u64,
    /// The game's own copy of the loader (`bundled/`) exists and is not the installed one.
    restore_ok: bool,
    /// What the card title says about the installed file (a stale sidecar is not believed).
    installed: Option<loader::Info>,
    job: Option<Job>,
    /// Set by the worker when the job ended: Ok(toast text) / Err(reason).
    result: Option<Result<String, String>>,
}

pub struct LoaderUpdates {
    /// A test folder instead of GitHub (env `RAC_LOADER_RELEASES`): `latest.json` and the release files directly in it.
    pub releases_dir: Option<String>,
    /// The (exe, game version) the last check was started for.
    asked_for: Option<(PathBuf, String)>,
    shared: Arc<Mutex<Shared>>,
}

/// What the card draws, for one exe.
pub(super) struct View {
    pub(super) line: Line,
    pub(super) offer: Option<Offer>,
    pub(super) restore_ok: bool,
    job: Option<(&'static str, Arc<Transfer>)>,
    info: Option<loader::Info>,
    /// No verdict for this exe yet (the update button waits for it).
    pub(super) checking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Line {
    None,
    Muted(String),
    Orange(String),
}

fn info_of(inst: &lu::Installed) -> loader::Info {
    loader::Info { present: inst.present, version: inst.version.clone(), abi: inst.abi.map(|a| a.text()) }
}

impl LoaderUpdates {
    pub fn from_env() -> LoaderUpdates {
        let dir = std::env::var("RAC_LOADER_RELEASES").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let shared = Shared { check: Check::Idle, for_exe: None, serial: 0, restore_ok: false, installed: None, job: None, result: None };
        LoaderUpdates { releases_dir: dir, asked_for: None, shared: Arc::new(Mutex::new(shared)) }
    }

    fn source(&self) -> ReleaseSource {
        ReleaseSource::from_override(self.releases_dir.as_deref(), lu::REPO)
    }

    /// Asks for the latest release unless that was done already for this exe and game version.
    pub fn ensure_checked(&mut self, root: &Path, exe: &Path, game_version: &str) {
        if !matches!(&self.asked_for, Some((e, v)) if e == exe && v == game_version) {
            self.start_check(root, exe, game_version);
        }
    }

    /// Looks again (after a change); a check still running for the same or another exe is dropped when this one starts.
    pub fn recheck(&mut self, root: &Path, exe: &Path, game_version: &str) {
        self.start_check(root, exe, game_version);
    }

    fn start_check(&mut self, root: &Path, exe: &Path, game_version: &str) {
        let serial = {
            let Ok(mut g) = self.shared.lock() else { return };
            g.serial += 1;
            g.check = Check::Checking;
            if g.for_exe.as_deref() != Some(exe) {
                g.restore_ok = false;
                g.installed = None;
            }
            g.for_exe = Some(exe.to_path_buf());
            g.serial
        };
        self.asked_for = Some((exe.to_path_buf(), game_version.to_string()));
        let (shared, src) = (self.shared.clone(), self.source());
        let (root, exe, gv) = (root.to_path_buf(), exe.to_path_buf(), game_version.to_string());
        std::thread::spawn(move || run_check(&shared, &src, &root, &exe, &gv, serial));
    }

    /// Something runs in the background (the window keeps repainting).
    pub fn busy(&self) -> bool {
        self.shared.lock().map(|g| matches!(g.check, Check::Checking) || g.job.is_some()).unwrap_or(false)
    }

    /// A change (install, go back, own loader) is running or its result has not been taken yet.
    pub fn job_running(&self) -> bool {
        self.shared.lock().map(|g| g.job.is_some()).unwrap_or(false)
    }

    /// Asks the running change to stop (a download stops within a moment; the loader stays as it was).
    pub fn cancel_job(&self) {
        if let Ok(g) = self.shared.lock() {
            if let Some(j) = &g.job {
                j.transfer.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    /// What the card shows for `exe`: another exe than the one the last verdict was made for gets "Looking..." and no offer.
    pub(super) fn view(&self, exe: &Path) -> Option<View> {
        let g = self.shared.lock().ok()?;
        let mine = g.for_exe.as_deref() == Some(exe);
        let (line, offer, checking) = match (&g.check, mine) {
            (Check::Done(v), true) => match &**v {
                Verdict::UpToDate(_) => (Line::Muted("The loader is up to date.".to_string()), None, false),
                Verdict::Offer(o) => (Line::None, Some(o.clone()), false),
                Verdict::Blocked(why) => (Line::Orange(why.clone()), None, false),
            },
            (Check::Failed(t), true) => (Line::Muted(t.clone()), None, false),
            _ => (Line::Muted("Looking for a newer loader...".to_string()), None, true),
        };
        let job = g.job.as_ref().map(|j| (j.what, j.transfer.clone()));
        Some(View { line, offer, restore_ok: mine && g.restore_ok, job, info: if mine { g.installed.clone() } else { None }, checking })
    }

    fn start_job(&self, what: &'static str, work: impl FnOnce(&Transfer) -> Result<String, String> + Send + 'static) {
        let transfer = Transfer::new();
        if let Ok(mut g) = self.shared.lock() {
            if g.job.is_some() {
                return;
            }
            g.job = Some(Job { what, transfer: transfer.clone() });
            g.result = None;
        }
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let r = work(&transfer);
            if let Ok(mut g) = shared.lock() {
                g.result = Some(r);
            }
        });
    }

    /// The finished job's outcome, once; the job is gone afterwards.
    fn take_result(&self) -> Option<Result<String, String>> {
        let mut g = self.shared.lock().ok()?;
        let r = g.result.take()?;
        g.job = None;
        Some(r)
    }
}

static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The worker of one check. The local answers (is the game's own loader different, what is installed) are stored before the network is
/// touched, and every store is skipped when a newer check has started since (`serial`).
fn run_check(shared: &Mutex<Shared>, src: &ReleaseSource, root: &Path, exe: &Path, gv: &str, serial: u64) {
    let restore_ok = restore_possible(root, exe);
    let first = info_of(&lu::installed(exe));
    if let Ok(mut g) = shared.lock() {
        if g.serial == serial {
            g.restore_ok = restore_ok;
            g.installed = Some(first);
        }
    }
    // a folder of its own per check (checks of two installs, or of a test next door, never share or remove each other's)
    let tmp = std::env::temp_dir().join(format!("rac-rs-launcher-loader-{}-{}", std::process::id(), TMP_SEQ.fetch_add(1, Ordering::Relaxed)));
    let mut now = None;
    let check = match lu::fetch(src, &tmp) {
        Ok((rel, man)) => match lu::host_abi(exe, gv) {
            Ok(host) => {
                let inst = lu::installed(exe);
                let v = lu::evaluate(&rel, &man, host, &inst, cfg!(windows), std::env::consts::ARCH);
                now = Some(info_of(&inst));
                Check::Done(Box::new(v))
            }
            Err(why) => Check::Failed(why),
        },
        Err(releases::ReleaseError::NotFound) => Check::Failed("No loader release has been published yet.".to_string()),
        Err(e) => Check::Failed(e.text()),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    if let Ok(mut g) = shared.lock() {
        if g.serial == serial {
            g.check = check;
            if now.is_some() {
                g.installed = now;
            }
        }
    }
}

/// The game's own loader exists in `bundled/` and is not the file next to the exe.
fn restore_possible(root: &Path, exe: &Path) -> bool {
    let bundled = lu::bundled_lib(root, cfg!(windows));
    if !bundled.is_file() {
        return false;
    }
    match (releases::sha256_file(&bundled), releases::sha256_file(&loader::lib_path(exe))) {
        (Ok(a), Ok(b)) => a != b,
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

/// "OmniLoader 0.1.0 (interface 1.6)" for the card title.
fn title_text(info: &loader::Info) -> String {
    let name = if cfg!(windows) { "omniloader.dll" } else { "libomniloader.so" };
    match (&info.version, &info.abi) {
        (Some(v), Some(a)) => format!("Mod loader: OmniLoader {v} (interface {a})"),
        (Some(v), None) => format!("Mod loader: OmniLoader {v}"),
        _ => format!("Mod loader: {name} (version unknown)"),
    }
}

fn release_url(o: &Offer) -> String {
    format!("https://github.com/{}/{}/releases/tag/{}", lu::REPO.0, lu::REPO.1, o.tag)
}

impl App {
    /// The game exe when the card applies: the game is installed, it is 0.4.2 or newer (older ones cannot say which interface they serve,
    /// and the Setup of 0.4.0 / 0.4.1 would overwrite a newer loader), it knows script mods and a loader library is next to the exe.
    fn loader_card_exe(&self) -> Option<PathBuf> {
        if !self.info.installed || !gating::available(&self.info.local_version, gating::SCRIPT_MODS) || !lu::supported_game(&self.info.local_version) {
            return None;
        }
        let exe = self.rac1().exe_path(&self.root);
        (exe.is_file() && loader::info(&exe).present).then_some(exe)
    }

    /// Takes in a finished change (called every frame from `mods_poll`): a toast, then the releases are looked at again.
    pub(super) fn mods_loader_poll(&mut self) {
        let Some(res) = self.loader_upd.take_result() else { return };
        match res {
            Ok(text) => self.toast(text, false),
            Err(why) => self.toast(why, true),
        }
        if let Some(exe) = self.loader_card_exe() {
            let (root, gv) = (self.root.clone(), self.info.local_version.clone());
            self.loader_upd.recheck(&root, &exe, &gv);
        }
    }

    /// The card under the two master ticks.
    pub fn mods_loader_card(&mut self, ui: &mut Ui) {
        let Some(exe) = self.loader_card_exe() else { return };
        let (root, gv) = (self.root.clone(), self.info.local_version.clone());
        self.loader_upd.ensure_checked(&root, &exe, &gv);
        let Some(v) = self.loader_upd.view(&exe) else { return };
        let info = v.info.clone().unwrap_or_else(|| loader::info(&exe));
        // the buttons that only touch files next to the game never wait for the network check; "Update loader" waits for its verdict
        let local_free = v.job.is_none() && !self.read_only() && !self.busy();
        let update_free = local_free && !v.checking;
        let can_back = lu::can_roll_back(&exe);
        let mut act = LoaderAct::None;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold(title_text(&info), 15.5));
            if let Some((what, t)) = &v.job {
                let (done, total) = (t.done.load(Ordering::Relaxed), t.total.load(Ordering::Relaxed));
                if total > 0 {
                    theme::progress(ui, done as f32 / total as f32, 8.0, theme::BLUE);
                } else {
                    theme::progress_busy(ui, 8.0, theme::BLUE);
                }
                ui.label(theme::muted(format!("{what}...")));
                if t.cancel.load(Ordering::Relaxed) {
                    ui.label(theme::muted("Cancelling and cleaning up..."));
                } else if theme::button_enabled(ui, "Cancel", true) {
                    act = LoaderAct::Cancel;
                }
                return;
            }
            if let Some(o) = &v.offer {
                ui.horizontal_wrapped(|ui| {
                    theme::chip(ui, &format!("Version {} is available", o.version), theme::GREEN);
                    if theme::button_enabled(ui, "Update loader", update_free) {
                        act = LoaderAct::Update;
                    }
                    if theme::link(ui, "What's new") {
                        act = LoaderAct::Notes;
                    }
                });
                ui.label(theme::muted(format!("Comes from github.com/{}/{} (interface {}, checked against this game).", lu::REPO.0, lu::REPO.1, o.abi.text())));
            }
            match &v.line {
                Line::Muted(t) => {
                    ui.label(theme::muted(t));
                }
                Line::Orange(t) => {
                    ui.label(RichText::new(t).color(theme::ORANGE).size(13.0));
                }
                Line::None => {}
            }
            if can_back || v.restore_ok {
                ui.horizontal_wrapped(|ui| {
                    if can_back && theme::button_enabled(ui, "Go back to the previous loader", local_free) {
                        act = LoaderAct::Back;
                    }
                    if v.restore_ok && theme::button_enabled(ui, "Use the game's own loader", local_free) {
                        act = LoaderAct::Own;
                    }
                });
            }
            if (v.offer.is_some() || can_back || v.restore_ok) && self.read_only() {
                ui.label(theme::muted("Close the game to change the loader."));
            }
        });
        self.loader_act(act, v.offer, &exe, &root);
        if self.loader_upd.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
    }

    fn loader_act(&mut self, act: LoaderAct, offer: Option<Offer>, exe: &Path, root: &Path) {
        let windows = cfg!(windows);
        let running = {
            let e = exe.to_path_buf();
            move || lu::exe_in_use_settled(&e)
        };
        match act {
            LoaderAct::None => {}
            LoaderAct::Notes => {
                if let Some(o) = &offer {
                    platform::open_url(&release_url(o));
                }
            }
            LoaderAct::Cancel => self.loader_upd.cancel_job(),
            LoaderAct::Update => {
                let Some(o) = offer else { return };
                let (src, exe, gv) = (self.loader_upd.source(), exe.to_path_buf(), self.info.local_version.clone());
                self.loader_upd.start_job("Downloading the new loader", move |t| {
                    // the verdict can be old: the game and the installed file are looked at again before anything is downloaded
                    let host = lu::host_abi(&exe, &gv)?;
                    lu::still_offered(&o, host, &lu::installed(&exe))?;
                    lu::install(&src, &o, &exe, t, &running, windows).map(|_| format!("The mod loader was updated to {}.", o.version))
                });
            }
            LoaderAct::Back => {
                let exe = exe.to_path_buf();
                self.loader_upd.start_job("Putting the previous loader back", move |_| {
                    lu::roll_back(&exe, &running).map(|_| "The previous mod loader is back.".to_string())
                });
            }
            LoaderAct::Own => {
                let (exe, root) = (exe.to_path_buf(), root.to_path_buf());
                self.loader_upd.start_job("Putting the game's own loader back", move |_| {
                    lu::restore_bundled(&root, &exe, &running, windows).map(|changed| {
                        if changed { "The game's own mod loader is back.".to_string() } else { "The game's own mod loader is already in use.".to_string() }
                    })
                });
            }
        }
    }
}

enum LoaderAct {
    None,
    Update,
    Notes,
    Cancel,
    Back,
    Own,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_version_and_interface() {
        let mut i = loader::Info { present: true, version: None, abi: None };
        assert!(title_text(&i).contains("version unknown"));
        i.version = Some("0.1.0".into());
        assert_eq!(title_text(&i), "Mod loader: OmniLoader 0.1.0");
        i.abi = Some("1.6".into());
        assert_eq!(title_text(&i), "Mod loader: OmniLoader 0.1.0 (interface 1.6)");
    }

    #[test]
    fn a_finished_job_is_taken_once_and_frees_the_slot() {
        let u = LoaderUpdates::from_env();
        u.start_job("Testing", |_| Ok("done".to_string()));
        for _ in 0..200 {
            if let Some(r) = u.take_result() {
                assert_eq!(r, Ok("done".to_string()));
                assert!(!u.busy());
                assert!(u.take_result().is_none());
                u.start_job("Again", |_| Err("no".to_string()));
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the job did not finish");
    }

    #[test]
    fn only_one_job_runs_at_a_time() {
        let u = LoaderUpdates::from_env();
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let g2 = gate.clone();
        u.start_job("First", move |_| {
            while !g2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok("one".to_string())
        });
        u.start_job("Second", |_| Ok("two".to_string()));
        gate.store(true, Ordering::Relaxed);
        for _ in 0..400 {
            if let Some(r) = u.take_result() {
                assert_eq!(r, Ok("one".to_string()));
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the job did not finish");
    }

    #[test]
    fn the_release_page_is_the_tag_of_the_offer() {
        let o = Offer {
            version: "0.2.0".into(),
            abi: lu::Abi { major: 1, minor: 6 },
            min_host: lu::Abi { major: 1, minor: 0 },
            tag: "v0.2.0".into(),
            file: lu::FileEntry { name: "x".into(), install_as: "omniloader.dll".into(), size: 1, sha256: String::new() },
            asset: releases::Asset { name: "x".into(), size: 1, browser_download_url: String::new(), digest: None },
            notes: String::new(),
        };
        assert_eq!(release_url(&o), "https://github.com/AsobiCodes/omniloader/releases/tag/v0.2.0");
    }

    // ------------------------------------------------------------------ a verdict belongs to the install it was made for

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rac_plu_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A "game" folder: a script that prints the host line after `delay` seconds (the stand-in for `rac-rs omni-host-abi`) and a library next to it.
    fn fake_game(dir: &Path, lib: &[u8], delay: u32) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let line = r#"{"omni_host":{"abi":"1.6","size":360}}"#;
        #[cfg(windows)]
        let exe = {
            let p = dir.join("fake.cmd");
            let wait = if delay > 0 { format!("ping -n {} 127.0.0.1 >nul\r\n", delay + 1) } else { String::new() };
            std::fs::write(&p, format!("@echo off\r\n{wait}echo {line}\r\n")).unwrap();
            p
        };
        #[cfg(unix)]
        let exe = {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join("fake.sh");
            std::fs::write(&p, format!("#!/bin/sh\nsleep {delay}\necho '{line}'\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            p
        };
        std::fs::write(loader::lib_path(&exe), lib).unwrap();
        exe
    }

    /// A release folder (the test source of `releases.rs`) holding loader `version` with the library `lib`.
    fn release_dir(dir: &Path, version: &str, lib: &[u8]) {
        let file = if cfg!(windows) { "omniloader-windows-x86_64.dll" } else { "libomniloader-linux-x86_64.so" };
        let key = lu::platform_key(cfg!(windows), "x86_64").unwrap();
        let sha: String = {
            use sha2::{Digest, Sha256};
            Sha256::digest(lib).iter().map(|b| format!("{b:02x}")).collect()
        };
        let man = serde_json::json!({
            "format": 1, "name": "omniloader", "version": version, "abi": "1.6", "min_host_abi": "1.0",
            "files": { key: { "name": file, "install_as": loader::lib_name_for(cfg!(windows)), "size": lib.len(), "sha256": sha } }
        })
        .to_string();
        releases::write_test_release(dir, &format!("v{version}"), "notes", &[(lu::MANIFEST_ASSET, man.as_bytes()), (file, lib)], &|_, h| Some(format!("sha256:{h}")));
    }

    fn updates_for(rel: &Path) -> LoaderUpdates {
        let mut u = LoaderUpdates::from_env();
        u.releases_dir = Some(rel.to_string_lossy().to_string());
        u
    }

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..1000 {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for: {what}");
    }

    #[test]
    fn another_install_never_shows_the_verdict_made_for_the_first_one() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let d = scratch("view");
        let (old, new) = (vec![1u8; 64], vec![2u8; 64]);
        release_dir(&d.join("rel"), "0.2.0", &new);
        let a = fake_game(&d.join("a"), &old, 0);
        let b = fake_game(&d.join("b"), &new, 0);
        let mut u = updates_for(&d.join("rel"));
        // before any check nothing is offered for any exe
        assert!(u.view(&a).unwrap().checking && u.view(&a).unwrap().offer.is_none());
        u.ensure_checked(&d, &a, "0.4.2");
        wait_for("the check for A", || !u.view(&a).unwrap().checking);
        let va = u.view(&a).unwrap();
        assert_eq!(va.offer.as_ref().map(|o| o.version.as_str()), Some("0.2.0"), "{:?}", va.line);
        // the player opens install B: A's offer is gone at once, B waits for its own check
        u.ensure_checked(&d, &b, "0.4.2");
        let still = u.view(&a).unwrap();
        assert!(still.offer.is_none() && still.checking && !still.restore_ok);
        assert!(u.view(&b).unwrap().offer.is_none());
        wait_for("the check for B", || !u.view(&b).unwrap().checking);
        let vb = u.view(&b).unwrap();
        assert_eq!(vb.line, Line::Muted("The loader is up to date.".to_string()));
        assert!(vb.offer.is_none());
        // and A is never shown B's verdict or its own old one; install C (old loader) gets its own offer, nobody else sees it
        assert!(u.view(&a).unwrap().offer.is_none());
        let c = fake_game(&d.join("c"), &old, 0);
        u.ensure_checked(&d, &c, "0.4.2");
        wait_for("the check for C", || !u.view(&c).unwrap().checking);
        assert!(u.view(&c).unwrap().offer.is_some());
        assert!(u.view(&a).unwrap().offer.is_none() && u.view(&b).unwrap().offer.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_answer_that_arrives_after_a_newer_check_began_is_dropped() {
        if std::env::consts::ARCH != "x86_64" {
            return;
        }
        let d = scratch("late");
        let (old, new) = (vec![1u8; 64], vec![2u8; 64]);
        release_dir(&d.join("rel"), "0.2.0", &new);
        // A answers slowly (the game takes about 2 s to say which interface it serves), B at once
        let a = fake_game(&d.join("a"), &old, 2);
        let b = fake_game(&d.join("b"), &new, 0);
        let mut u = updates_for(&d.join("rel"));
        u.ensure_checked(&d, &a, "0.4.2");
        u.ensure_checked(&d, &b, "0.4.2");
        wait_for("the check for B", || !u.view(&b).unwrap().checking);
        assert_eq!(u.view(&b).unwrap().line, Line::Muted("The loader is up to date.".to_string()));
        // A's check ends now (an offer for A): it must not overwrite B's verdict
        std::thread::sleep(Duration::from_millis(3500));
        let vb = u.view(&b).unwrap();
        assert!(vb.offer.is_none() && !vb.checking, "B shows A's late answer");
        assert_eq!(vb.line, Line::Muted("The loader is up to date.".to_string()));
        // checking A again works and gives A's own verdict
        u.recheck(&d, &a, "0.4.2");
        wait_for("the second check for A", || !u.view(&a).unwrap().checking);
        assert!(u.view(&a).unwrap().offer.is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_local_button_is_known_before_the_network_answers() {
        // restore_ok is stored before the release is fetched: a release folder that does not exist yet still gets it
        let d = scratch("local");
        let root = d.join("root");
        std::fs::create_dir_all(root.join("bundled")).unwrap();
        std::fs::write(lu::bundled_lib(&root, cfg!(windows)), vec![9u8; 64]).unwrap();
        let exe = fake_game(&d.join("g"), &vec![1u8; 64], 0);
        let mut u = updates_for(&d.join("no_release_here"));
        u.ensure_checked(&root, &exe, "0.4.2");
        wait_for("the check", || !u.view(&exe).unwrap().checking);
        let v = u.view(&exe).unwrap();
        assert!(v.restore_ok, "the game's own loader can be put back");
        assert!(matches!(v.line, Line::Muted(_)) && v.offer.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_running_loader_change_blocks_switching_installs_and_play_until_it_is_taken_in() {
        let home = scratch("app");
        let mut app = App::new(home.join("data"), None);
        std::fs::create_dir_all(home.join("other")).unwrap();
        let other = app.library.add(crate::games::game_by_id("rac1").unwrap_or(&crate::games::GAMES[0]), &home.join("other"));
        let first = app.cur_id.clone();
        let gate = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let g2 = gate.clone();
        app.loader_upd.start_job("Testing", move |_| {
            while !g2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok("done".to_string())
        });
        assert!(app.loader_upd.job_running() && app.busy());
        assert!(!app.select_install(&other.id), "switching installs while the loader is being changed");
        assert_eq!(app.cur_id, first);
        app.play(&egui::Context::default());
        assert!(app.play_error.as_deref().unwrap_or("").contains("mod loader is being changed"), "{:?}", app.play_error);
        // the job ends: still busy until the UI takes the result in
        gate.store(true, Ordering::Relaxed);
        wait_for("the job result", || app.loader_upd.shared.lock().map(|g| g.result.is_some()).unwrap_or(false));
        assert!(app.busy());
        app.mods_loader_poll();
        assert!(!app.busy());
        assert!(app.select_install(&other.id));
        assert_ne!(app.cur_id, first);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cancel_job_sets_the_cancel_flag_of_the_running_job_only() {
        let u = LoaderUpdates::from_env();
        u.cancel_job(); // nothing running: nothing happens
        let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = seen.clone();
        u.start_job("Testing", move |t| {
            for _ in 0..500 {
                if t.cancel.load(Ordering::Relaxed) {
                    s2.store(true, Ordering::Relaxed);
                    return Err("Cancelled.".to_string());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok("never cancelled".to_string())
        });
        u.cancel_job();
        wait_for("the cancelled job", || u.take_result().is_some());
        assert!(seen.load(Ordering::Relaxed));
    }
}
