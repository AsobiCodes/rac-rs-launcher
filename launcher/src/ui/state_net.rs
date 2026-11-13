//! The network flows of the launcher: the game download for a new install and the launcher's own update.
//! (Logic lives in `game_dl.rs`, `selfupdate.rs`, `releases.rs`; this file ties them to the `App`. Drawing is in `page_net.rs`.)
//! Every network failure ends as a plain message; offline or a missing repo never blocks the launcher.
use super::state::{App, Sel};
use crate::game_dl::GameDl;
use crate::releases::{self, ReleaseError, ReleaseSource, Transfer};
use crate::selfupdate::{self, SelfState};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// A running launcher update (download + swap).
pub struct SelfApply {
    pub transfer: Arc<Transfer>,
    pub started: Instant,
    pub result: Arc<Mutex<Option<Result<PathBuf, ReleaseError>>>>,
}

/// The release list behind the version picker.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum VersionList {
    #[default]
    Idle,
    Loading,
    Ready(Vec<crate::releases::Release>),
    Failed(String),
    /// The repo or its release does not exist (HTTP 404): the game is not released yet.
    NotReleased,
}

pub struct Net {
    /// Where the game zip comes from: GitHub, or a test folder (env `RAC_GAME_RELEASES`, flag `--game-releases`).
    pub game_src: ReleaseSource,
    /// Where launcher updates come from (env `RAC_LAUNCHER_RELEASES`, flag `--launcher-releases`).
    pub launcher_src: ReleaseSource,
    /// The game the "New install" form is for (`rac1` / `rac2`): its repo, zip name and folder name.
    pub new_game: &'static str,
    /// The game of the running download.
    pub dl_game: &'static str,
    pub game_dl: Option<GameDl>,
    pub dl_error: Option<String>,
    pub self_state: Arc<Mutex<SelfState>>,
    /// The running launcher's exe as it was at start (after a rename `current_exe()` may name the `.old` file).
    pub self_exe: PathBuf,
    pub self_apply: Option<SelfApply>,
    pub self_error: Option<String>,
    pub notes_open: bool,
    /// The running download is for an older version: pin the new entry when it lands.
    pub pin_after: bool,
    /// The releases of the game repo for the version picker (loaded when the "New install" form opens).
    pub versions: Arc<Mutex<VersionList>>,
    /// "Show test versions" tick of the picker.
    pub show_test: bool,
    /// The version picked for a new install (None = the newest).
    pub pick: Option<String>,
    /// Tests: do not start the new exe, only note it in `restart_log`.
    pub dry_restart: bool,
    pub restart_log: Option<PathBuf>,
}

impl Net {
    pub fn from_env() -> Net {
        let env = |k: &str| std::env::var(k).ok();
        Net {
            game_src: ReleaseSource::from_override(env("RAC_GAME_RELEASES").as_deref(), releases::GAME_REPO),
            launcher_src: ReleaseSource::from_override(env("RAC_LAUNCHER_RELEASES").as_deref(), releases::LAUNCHER_REPO),
            new_game: "rac1",
            dl_game: "rac1",
            game_dl: None,
            dl_error: None,
            self_state: Arc::new(Mutex::new(SelfState::Idle)),
            self_exe: std::env::current_exe().unwrap_or_default(),
            self_apply: None,
            self_error: None,
            notes_open: false,
            pin_after: false,
            versions: Arc::new(Mutex::new(VersionList::Idle)),
            show_test: false,
            pick: None,
            dry_restart: std::env::var("RAC_LAUNCHER_DRY_RESTART").map(|v| v == "1").unwrap_or(false),
            restart_log: None,
        }
    }
}

/// The plain message for a game whose repo or release does not exist yet (HTTP 404).
pub fn not_released_text(game: &crate::games::GameDef) -> String {
    format!("{} has not been released yet, so there is nothing to download for it right now. Nothing was changed.", game.title)
}

impl App {
    // ------------------------------------------------------------------ game download

    pub fn downloading(&self) -> bool {
        self.net.game_dl.is_some()
    }

    /// The game of the "New install" form.
    pub fn new_game_def(&self) -> &'static crate::games::GameDef {
        crate::games::game_by_id(self.net.new_game).unwrap_or(&crate::games::GAMES[0])
    }

    /// Where the zip of `game` comes from: its own repo; a test folder (flag or env) stands in for every game.
    pub fn game_source(&self, game: &crate::games::GameDef) -> ReleaseSource {
        match &self.net.game_src {
            ReleaseSource::Folder(_) => self.net.game_src.clone(),
            _ if game.id == "rac1" => self.net.game_src.clone(),
            _ => ReleaseSource::github(game.repo),
        }
    }

    /// The form switches to another game: its own version list, pick and (when the folder is still the suggested one) folder name.
    pub fn set_new_game(&mut self, id: &str) {
        let Some(g) = crate::games::game_by_id(id).filter(|g| g.supported && g.visible) else { return };
        if g.id == self.net.new_game {
            return;
        }
        self.net.new_game = g.id;
        self.net.pick = None;
        if let Ok(mut v) = self.net.versions.lock() {
            *v = VersionList::Idle;
        }
    }

    /// Starts the download of the latest game release into `dest` (a missing or empty folder). The caller checked the space.
    pub fn start_game_download(&mut self, dest: PathBuf, want: Option<String>) -> Result<String, String> {
        if self.net.game_dl.is_some() {
            return Err("A download is already running.".to_string());
        }
        if !crate::game_dl::folder_is_free(&dest) {
            return Err("That folder is not empty. Choose an empty folder or a new one, so nothing of yours is mixed up with the game.".to_string());
        }
        self.net.dl_error = None;
        // an explicit older version = a pinned install (the newest stays unpinned)
        let newest = self.installable_versions().first().map(|i| i.version.clone());
        self.net.pin_after = want.is_some() && want != newest;
        let game = self.new_game_def();
        self.net.dl_game = game.id;
        self.net.game_dl = Some(GameDl::start_game(game, self.game_source(game), dest.clone(), want));
        Ok(format!("Downloading the game into {}", dest.display()))
    }

    /// Starts loading the release list for the picker (once; `force` reloads, e.g. after a failure).
    pub fn load_versions(&mut self, force: bool) {
        {
            let Ok(mut g) = self.net.versions.lock() else { return };
            if matches!(*g, VersionList::Loading | VersionList::Ready(_)) && !force {
                return;
            }
            *g = VersionList::Loading;
        }
        let (src, slot) = (self.game_source(self.new_game_def()), self.net.versions.clone());
        std::thread::spawn(move || {
            let r = match src.list() {
                Ok(l) => VersionList::Ready(l),
                Err(ReleaseError::NotFound) => VersionList::NotReleased,
                Err(e) => VersionList::Failed(e.text()),
            };
            if let Ok(mut g) = slot.lock() {
                *g = r;
            }
        });
    }

    pub fn version_list(&self) -> VersionList {
        self.net.versions.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// What the picker can offer (empty while loading or offline).
    pub fn installable_versions(&self) -> Vec<crate::game_dl::Installable> {
        match self.version_list() {
            VersionList::Ready(l) => crate::game_dl::installable_for_game(cfg!(windows), self.new_game_def(), &l, self.net.show_test),
            _ => Vec::new(),
        }
    }

    /// The version a new install gets now: the picked one if it is still in the list, else the default (newest).
    pub fn picked_version(&self) -> Option<String> {
        let v = self.installable_versions();
        match &self.net.pick {
            Some(p) if v.iter().any(|i| &i.version == p) => Some(p.clone()),
            _ => crate::game_dl::default_pick(&v).map(|i| i.version.clone()),
        }
    }

    /// True when the open install is pinned to an older version.
    pub fn is_pinned(&self) -> bool {
        self.cur_id.as_deref().and_then(|id| self.library.get(id)).map(|e| e.pinned).unwrap_or(false)
    }

    pub fn cancel_download(&mut self) {
        if let Some(d) = &self.net.game_dl {
            d.cancel();
        }
    }

    fn poll_game_dl(&mut self) {
        let Some(res) = self.net.game_dl.as_ref().and_then(|d| d.take_result()) else { return };
        let dest = self.net.game_dl.take().map(|d| d.dest).unwrap_or_default();
        match res {
            Ok(ver) => {
                let game = crate::games::game_by_id(self.net.dl_game).unwrap_or(&crate::games::GAMES[0]);
                let e = self.library.add(game, &dest);
                if self.net.pin_after {
                    self.library.set_pinned(&e.id, true);
                }
                self.net.pin_after = false;
                self.save_library();
                self.sel = Sel::Game(game.id);
                self.select_install(&e.id);
                let name = if game.id == "rac1" { "RAC-RS".to_string() } else { format!("RAC-RS {}", game.short.trim_start_matches("R&C ")) };
                self.toast(&format!("{name} {ver} is downloaded. Pick your disc to set the game up."), false);
            }
            Err(ReleaseError::Cancelled) => self.toast("The download was cancelled. Nothing was kept.", false),
            Err(ReleaseError::NotFound) if self.net.dl_game != "rac1" => {
                let g = crate::games::game_by_id(self.net.dl_game).unwrap_or(&crate::games::GAMES[0]);
                self.net.dl_error = Some(not_released_text(g));
            }
            Err(e) => self.net.dl_error = Some(e.text()),
        }
    }

    // ------------------------------------------------------------------ launcher self-update

    /// Looks for a newer launcher on a thread (called once at start by the real app; tests call it themselves).
    pub fn start_self_check(&mut self) {
        if std::env::var("RAC_LAUNCHER_NO_SELF_UPDATE").map(|v| v == "1").unwrap_or(false) {
            return;
        }
        selfupdate::check_async(self.net.launcher_src.clone(), selfupdate::OWN_VERSION.to_string(), self.net.self_state.clone());
    }

    pub fn self_state(&self) -> SelfState {
        self.net.self_state.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// "Update launcher": downloads and swaps the exe on a thread; `tick` restarts when it is done.
    pub fn apply_launcher_update(&mut self) {
        let SelfState::Available(upd) = self.self_state() else { return };
        if self.net.self_apply.is_some() {
            return;
        }
        if self.busy() || self.game_running() {
            self.toast("Wait until the setup or the game is finished, then update the launcher.", true);
            return;
        }
        self.net.self_error = None;
        let transfer = Transfer::new();
        let result = Arc::new(Mutex::new(None));
        self.net.self_apply = Some(SelfApply { transfer: transfer.clone(), started: Instant::now(), result: result.clone() });
        let (src, exe) = (self.net.launcher_src.clone(), self.net.self_exe.clone());
        std::thread::spawn(move || {
            let r = selfupdate::apply(&src, &upd, &exe, &transfer);
            if let Ok(mut g) = result.lock() {
                *g = Some(r);
            }
        });
    }

    fn poll_self_apply(&mut self) {
        let Some(res) = self.net.self_apply.as_ref().and_then(|a| a.result.lock().ok().and_then(|mut g| g.take())) else { return };
        self.net.self_apply = None;
        match res {
            Ok(exe) => self.restart_exe = Some(exe),
            Err(e) => self.net.self_error = Some(e.text()),
        }
    }

    /// The restart after a launcher update: start `exe --restarted` (the same flags as this start) and quit; a dry run only records it.
    pub fn do_restart(&mut self, exe: &Path) {
        if self.net.dry_restart {
            self.net.restart_log = Some(exe.to_path_buf());
            return;
        }
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--restarted").current_dir(exe.parent().unwrap_or(Path::new(".")));
        if self.forced_root {
            cmd.arg("--root").arg(&self.root);
        }
        if let Some(b) = &self.update_base {
            cmd.arg("--update-base").arg(b);
        }
        if cmd.spawn().is_ok() {
            self.quit_now = true;
        }
    }

    /// Part of `tick`: results of the two network jobs. True while one runs.
    pub fn poll_net(&mut self) -> bool {
        self.poll_game_dl();
        self.poll_self_apply();
        self.net.game_dl.is_some() || self.net.self_apply.is_some() || matches!(self.self_state(), SelfState::Checking)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_net_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }
    fn wait(app: &mut App, what: impl Fn(&App) -> bool) {
        for _ in 0..300 {
            app.poll_net();
            if what(app) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("timeout");
    }

    #[test]
    fn launcher_update_end_to_end_with_dry_restart() {
        let rel = scratch("lrel");
        let home = scratch("lhome");
        let new_bytes = selfupdate::test_launcher_bytes(b"NEW LAUNCHER");
        releases::write_test_release(&rel, "v9.0.0", "- shiny", &[(selfupdate::ASSET_NAME, new_bytes.as_slice())], &good);
        let exe = home.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLD LAUNCHER").unwrap();
        let mut app = App::new(home.join("data"), None);
        app.net.launcher_src = ReleaseSource::Folder(rel.clone());
        app.net.self_exe = exe.clone();
        app.net.dry_restart = true;
        app.start_self_check();
        wait(&mut app, |a| matches!(a.self_state(), SelfState::Available(_)));
        assert!(matches!(app.self_state(), SelfState::Available(u) if u.version == "9.0.0" && u.notes == "- shiny"));
        app.apply_launcher_update();
        wait(&mut app, |a| a.restart_exe.is_some() || a.net.self_error.is_some());
        assert_eq!(app.net.self_error, None);
        let exe2 = app.restart_exe.take().unwrap();
        app.do_restart(&exe2);
        assert_eq!(app.net.restart_log.as_deref(), Some(exe.as_path()));
        assert_eq!(std::fs::read(&exe).unwrap(), new_bytes);
        assert_eq!(std::fs::read(selfupdate::old_path(&exe)).unwrap(), b"OLD LAUNCHER");
        // the next start deletes the old exe next to the exe, not in the data folder
        assert!(selfupdate::remove_old(&exe) && !selfupdate::old_path(&exe).exists());
        assert!(!app.quit_now);
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn failed_launcher_update_shows_an_error_and_keeps_the_exe() {
        let rel = scratch("frel");
        let home = scratch("fhome");
        releases::write_test_release(&rel, "v9.0.0", "", &[(selfupdate::ASSET_NAME, b"NEW")], &|_, _| None);
        let exe = home.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLD").unwrap();
        let mut app = App::new(home.join("data"), None);
        app.net.launcher_src = ReleaseSource::Folder(rel.clone());
        app.net.self_exe = exe.clone();
        app.start_self_check();
        wait(&mut app, |a| matches!(a.self_state(), SelfState::Available(_)));
        app.apply_launcher_update();
        wait(&mut app, |a| a.net.self_error.is_some());
        assert!(app.net.self_error.as_deref().unwrap().contains("no checksum"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLD");
        assert!(app.restart_exe.is_none());
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn offline_or_missing_repo_is_silent() {
        let home = scratch("ohome");
        let mut app = App::new(home.join("data"), None);
        app.net.launcher_src = ReleaseSource::Folder(home.join("nothing"));
        app.start_self_check();
        wait(&mut app, |a| matches!(a.self_state(), SelfState::Unavailable(_)));
        assert!(app.net.self_error.is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn new_install_downloads_the_release_adds_it_and_opens_it() {
        let rel = scratch("grel");
        let home = scratch("ghome");
        let mut zip = std::io::Cursor::new(Vec::new());
        {
            use std::io::Write;
            let mut w = zip::ZipWriter::new(&mut zip);
            let o = zip::write::SimpleFileOptions::default();
            for (n, b) in [("rac-rs-9.9.9/VERSION", &b"9.9.9\n"[..]), ("rac-rs-9.9.9/setup.ps1", b"# s"), ("rac-rs-9.9.9/Play.cmd", b"x")] {
                w.start_file(n, o).unwrap();
                w.write_all(b).unwrap();
            }
            w.finish().unwrap();
        }
        releases::write_test_release(&rel, "v9.9.9", "", &[("rac-rs-9.9.9.zip", zip.get_ref())], &good);
        let mut app = App::new(home.join("data"), None);
        app.net.game_src = ReleaseSource::Folder(rel.clone());
        let dest = home.join("games").join("rac");
        let r = app.new_install(&dest.to_string_lossy());
        assert!(r.is_ok(), "{r:?}");
        assert!(app.downloading());
        // a second start while running is refused
        assert!(app.start_game_download(home.join("other"), None).is_err());
        wait(&mut app, |a| !a.downloading());
        assert_eq!(app.net.dl_error, None);
        assert!(dest.join("VERSION").is_file() && dest.join("setup.ps1").is_file());
        assert!(app.library.find_folder(&dest).is_some());
        assert_eq!(app.info.local_version, "9.9.9");
        assert!(!app.info.installed); // the disc setup comes next
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn picking_an_older_version_pins_the_new_entry_and_pin_survives_a_restart() {
        let rel = scratch("prel");
        let home = scratch("phome");
        let zip_for = |ver: &str| {
            let mut z = std::io::Cursor::new(Vec::new());
            {
                use std::io::Write;
                let mut w = zip::ZipWriter::new(&mut z);
                let o = zip::write::SimpleFileOptions::default();
                for (n, b) in [("VERSION", format!("{ver}\n")), ("setup.ps1", "# s".to_string())] {
                    w.start_file(format!("rac-rs-{ver}/{n}"), o).unwrap();
                    w.write_all(b.as_bytes()).unwrap();
                }
                w.finish().unwrap();
            }
            z.into_inner()
        };
        let (z31, z22) = (zip_for("0.4.2"), zip_for("0.4.1"));
        releases::write_test_release(&rel, "v0.4.2", "", &[("rac-rs-0.4.2.zip", &z31)], &good);
        let a = std::fs::read_to_string(rel.join("latest.json")).unwrap();
        releases::write_test_release(&rel, "v0.4.1", "", &[("rac-rs-0.4.1.zip", &z22)], &good);
        let b = std::fs::read_to_string(rel.join("latest.json")).unwrap();
        std::fs::write(rel.join("releases.json"), format!("[{a},{b}]")).unwrap();
        let data = home.join("data");
        let mut app = App::new(data.clone(), None);
        app.net.game_src = ReleaseSource::Folder(rel.clone());
        app.load_versions(false);
        for _ in 0..200 {
            if matches!(app.version_list(), VersionList::Ready(_)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let v: Vec<String> = app.installable_versions().iter().map(|i| i.version.clone()).collect();
        assert_eq!(v, ["0.4.2", "0.4.1"]);
        assert_eq!(app.picked_version().as_deref(), Some("0.4.2")); // default = newest
        app.net.pick = Some("0.4.1".into());
        assert_eq!(app.picked_version().as_deref(), Some("0.4.1"));
        let dest = home.join("Ratchet & Clank 0.4.1");
        app.start_game_download(dest.clone(), app.picked_version()).unwrap();
        wait(&mut app, |a| !a.downloading());
        assert_eq!(app.info.local_version, "0.4.1");
        assert!(app.is_pinned());
        // restart: the pin is read back from library.json
        let lib = crate::library::Library::load(&data);
        assert!(lib.find_folder(&dest).unwrap().pinned);
        // the newest is not pinned
        let dest2 = home.join("newest");
        app.start_game_download(dest2.clone(), Some("0.4.2".into())).unwrap();
        wait(&mut app, |a| !a.downloading());
        assert!(!app.is_pinned());
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn download_errors_are_shown_and_a_full_folder_is_refused() {
        let home = scratch("ehome");
        let mut app = App::new(home.join("data"), None);
        app.net.game_src = ReleaseSource::Folder(home.join("no_release_here"));
        let dest = home.join("g");
        app.start_game_download(dest.clone(), None).unwrap();
        wait(&mut app, |a| !a.downloading());
        assert!(app.net.dl_error.as_deref().unwrap().contains("no release"));
        // a folder with other files in it is never unpacked into
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("mine.txt"), "x").unwrap();
        assert!(app.start_game_download(dest.clone(), None).unwrap_err().contains("not empty"));
        app.net.dl_error = None;
        let t = Transfer::default();
        t.cancel.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&home);
    }

    fn game_zip(prefix: &str, ver: &str, script: &str) -> Vec<u8> {
        use std::io::Write;
        let mut zip = std::io::Cursor::new(Vec::new());
        let mut w = zip::ZipWriter::new(&mut zip);
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in [("VERSION", format!("{ver}
")), ("setup.ps1", script.to_string()), ("Play.cmd", "x".to_string())] {
            w.start_file(format!("{prefix}-{ver}/{n}"), o).unwrap();
            w.write_all(b.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        zip.into_inner()
    }

    #[test]
    fn each_game_has_its_own_repo_and_zip_name() {
        let mut app = App::new(scratch("srchome").join("data"), None);
        let (r1, r2) = (crate::games::game_by_id("rac1").unwrap(), crate::games::game_by_id("rac2").unwrap());
        let is = |s: ReleaseSource, repo: &str| matches!(s, ReleaseSource::GitHub { owner, repo: r } if owner == "AsobiCodes" && r == repo);
        app.net.game_src = ReleaseSource::github(releases::GAME_REPO);
        assert!(is(app.game_source(r1), "rac-rs") && is(app.game_source(r2), "rac-rs2"));
        assert_eq!((r2.zip_prefix, r2.min_version), ("rac-rs2", "0.1.0"));
        // a test folder stands in for every game
        app.net.game_src = ReleaseSource::Folder(PathBuf::from("X:\rel"));
        assert!(matches!(app.game_source(r2), ReleaseSource::Folder(_)));
    }

    #[test]
    fn new_rac2_install_downloads_rac_rs2_zip_and_is_listed_as_rac2() {
        let (rel, home) = (scratch("r2rel"), scratch("r2home"));
        let zip = game_zip("rac-rs2", "0.1.0", "param([string]$Game = 'rac2')");
        releases::write_test_release(&rel, "v0.1.0", "", &[("rac-rs2-0.1.0.zip", &zip)], &good);
        let mut app = App::new(home.join("data"), None);
        app.net.game_src = ReleaseSource::Folder(rel.clone());
        app.set_new_game("rac2");
        app.load_versions(true);
        wait(&mut app, |a| !matches!(a.version_list(), VersionList::Loading));
        assert_eq!(app.installable_versions().iter().map(|i| i.version.clone()).collect::<Vec<_>>(), vec!["0.1.0"]);
        let dest = home.join("games").join("rac2");
        assert!(app.new_install(&dest.to_string_lossy()).is_ok());
        wait(&mut app, |a| !a.downloading());
        assert_eq!(app.net.dl_error, None);
        let e = app.library.find_folder(&dest).expect("listed").clone();
        assert_eq!(e.game, "rac2");
        assert!(dest.join("VERSION").is_file() && !dest.join("rac-rs2-0.1.0").exists());
        assert!(app.is_rac2());
        // the R&C 1 release feed (a rac-rs-<ver>.zip) is never taken for R&C 2
        let (rel1, home1) = (scratch("r2rel1"), scratch("r2home1"));
        releases::write_test_release(&rel1, "v9.9.9", "", &[("rac-rs-9.9.9.zip", &game_zip("rac-rs", "9.9.9", "# s"))], &good);
        let mut a1 = App::new(home1.join("data"), None);
        a1.net.game_src = ReleaseSource::Folder(rel1.clone());
        a1.set_new_game("rac2");
        a1.start_game_download(home1.join("g"), None).unwrap();
        wait(&mut a1, |a| !a.downloading());
        assert!(a1.net.dl_error.is_some() && a1.library.find_folder(&home1.join("g")).is_none(), "{:?}", a1.net.dl_error);
        for d in [rel, home, rel1, home1] {
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_missing_rac2_release_says_not_released_yet() {
        let home = scratch("r2miss");
        let mut app = App::new(home.join("data"), None);
        app.net.game_src = ReleaseSource::Folder(home.join("nothing_here"));
        app.set_new_game("rac2");
        app.load_versions(true);
        wait(&mut app, |a| !matches!(a.version_list(), VersionList::Loading));
        assert_eq!(app.version_list(), VersionList::NotReleased);
        app.start_game_download(home.join("g"), None).unwrap();
        wait(&mut app, |a| !a.downloading());
        let m = app.net.dl_error.clone().unwrap();
        assert!(m.contains("Ratchet & Clank 2 has not been released yet") && !m.contains("HTTP") && !m.contains("404"), "{m}");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Installs an R&C 2 0.1.0 from a test folder that is also the update base (VERSION = `feed`), returns the app.
    fn rac2_app_with_feed(tag: &str, feed: Option<&str>) -> (App, PathBuf, PathBuf) {
        let (rel, home) = (scratch(&format!("{tag}rel")), scratch(&format!("{tag}home")));
        let zip = game_zip("rac-rs2", "0.1.0", "param([string]$Game = 'rac2')");
        releases::write_test_release(&rel, "v0.1.0", "", &[("rac-rs2-0.1.0.zip", &zip)], &good);
        if let Some(f) = feed {
            std::fs::write(rel.join("VERSION"), format!("{f}\n")).unwrap();
            std::fs::write(rel.join("CHANGELOG.md"), format!("## v{f}\n- R2 fix\n\n## v0.1.0\n- first\n")).unwrap();
        }
        let mut app = App::new(home.join("data"), Some(rel.to_string_lossy().to_string()));
        app.net.game_src = ReleaseSource::Folder(rel.clone());
        app.set_new_game("rac2");
        let dest = home.join("games").join("rac2");
        app.start_game_download(dest.clone(), None).unwrap();
        wait(&mut app, |a| !a.downloading());
        assert_eq!(app.net.dl_error, None);
        assert!(app.library.find_folder(&dest).is_some());
        (app, rel, home)
    }

    /// Makes the open install look finished (game exe + generated\play.cfg) so that the update check runs.
    fn fake_installed(app: &mut App) {
        let (root, g) = (app.root.clone(), app.rac1());
        let exe = g.exe_path(&root);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"x").unwrap();
        std::fs::create_dir_all(root.join("generated")).unwrap();
        std::fs::write(root.join("generated").join("play.cfg"), "iso=none.iso
").unwrap();
        std::fs::write(root.join("generated").join("built_version.txt"), format!("{}
", crate::games::local_version(&root))).unwrap();
        app.refresh_info();
        assert!(app.info.installed);
    }

    fn settle(app: &mut App) -> crate::update::UpdateState {
        fake_installed(app);
        app.check_updates();
        for _ in 0..300 {
            let s = app.update_state();
            if s != crate::update::UpdateState::Checking {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("update check timeout");
    }

    #[test]
    fn rac2_install_is_offered_a_newer_rac2_release_and_only_that() {
        use crate::update::UpdateState as U;
        let (mut app, rel, home) = rac2_app_with_feed("r2up", Some("0.1.1"));
        assert!(app.is_rac2() && app.rac1().repo == releases::RAC2_REPO && app.rac1().repo != releases::GAME_REPO);
        match settle(&mut app) {
            U::Available(o) => assert_eq!((o.from.as_str(), o.to.as_str(), o.finish), ("0.1.0", "0.1.1", false)),
            s => panic!("{s:?}"),
        }
        // same version: nothing to offer
        std::fs::write(rel.join("VERSION"), "0.1.0\n").unwrap();
        assert_eq!(settle(&mut app), U::UpToDate("0.1.0".into()));
        // the feed does not exist (the repo has no VERSION / 404): silent, not "Could not check"
        std::fs::remove_file(rel.join("VERSION")).unwrap();
        assert_eq!(settle(&mut app), U::UpToDate("0.1.0".into()));
        for d in [rel, home] {
            let _ = std::fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn rac1_install_keeps_its_own_feed_and_offline_wording() {
        use crate::update::UpdateState as U;
        let (rel, home) = (scratch("r1up_rel"), scratch("r1up_home"));
        let zip = game_zip("rac-rs", "0.5.1", "param([string]$Game = 'rac1')");
        releases::write_test_release(&rel, "v0.5.1", "", &[("rac-rs-0.5.1.zip", &zip)], &good);
        std::fs::write(rel.join("VERSION"), "0.5.2\n").unwrap();
        let mut app = App::new(home.join("data"), Some(rel.to_string_lossy().to_string()));
        app.net.game_src = ReleaseSource::Folder(rel.clone());
        app.set_new_game("rac1");
        let dest = home.join("games").join("rac1");
        app.start_game_download(dest.clone(), None).unwrap();
        wait(&mut app, |a| !a.downloading());
        assert!(!app.is_rac2() && app.rac1().repo == releases::GAME_REPO && app.rac1().repo != releases::RAC2_REPO);
        assert!(matches!(settle(&mut app), U::Available(o) if o.to == "0.5.2"));
        // R&C 1 without a feed is still "offline" as before
        std::fs::remove_file(rel.join("VERSION")).unwrap();
        assert_eq!(settle(&mut app), U::Offline);
        for d in [rel, home] {
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}
