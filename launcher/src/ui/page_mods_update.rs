//! Mod updates on the Mods page: which script mods have a newer release in their own GitHub repo, the "Update to X" button on the row,
//! the progress card, and "Get Arsenal Mastery" when the mod is not installed. The logic (versions, download, unpack, swap) is in `mods/update.rs`.
//! One request per repo per launcher start: the answer is cached in `ModUpdates` (the Details button "Check for updates" asks again). A failed
//! check stays quiet on the row and only shows as a grey line in the Details; nothing here nags or blocks the page.
use super::page_mods_script::PanelState;
use super::state::App;
use super::theme;
use crate::gating;
use crate::mods::update::{self, Done, Offer, UpdateJob, UpdateSource, Verdict, FEATURED};
use crate::mods::ModEntry;
use crate::platform;
use crate::releases::{Release, ReleaseError};
use egui::Ui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What one repo's latest release answered.
pub enum Check {
    Checking,
    Done(Box<Release>),
    Failed(ReleaseError),
}

type Inbox = Arc<Mutex<Vec<(String, Result<Release, ReleaseError>)>>>;

pub struct ModUpdates {
    /// A test folder instead of GitHub (env `RAC_MOD_RELEASES`, flag `--mod-releases`): `<dir>/<owner>/<repo>/latest.json` and the zips.
    pub releases_dir: Option<PathBuf>,
    checks: HashMap<String, Check>,
    inbox: Inbox,
    pub job: Option<UpdateJob>,
}

impl ModUpdates {
    pub fn from_env() -> ModUpdates {
        let dir = std::env::var("RAC_MOD_RELEASES").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).map(PathBuf::from);
        ModUpdates { releases_dir: dir, checks: HashMap::new(), inbox: Inbox::default(), job: None }
    }

    /// Asks for the repo's latest release unless it was asked already (this launcher start).
    pub fn ensure_checked(&mut self, s: &UpdateSource) {
        let key = s.slug();
        if self.checks.contains_key(&key) {
            return;
        }
        self.checks.insert(key.clone(), Check::Checking);
        let (src, inbox) = (s.release_source(self.releases_dir.as_deref()), self.inbox.clone());
        std::thread::spawn(move || {
            let r = src.latest();
            if let Ok(mut g) = inbox.lock() {
                g.push((key, r));
            }
        });
    }

    /// Forgets the answer and asks again.
    pub fn recheck(&mut self, s: &UpdateSource) {
        if !matches!(self.checks.get(&s.slug()), Some(Check::Checking)) {
            self.checks.remove(&s.slug());
            self.ensure_checked(s);
        }
    }

    pub fn check(&self, s: &UpdateSource) -> Option<&Check> {
        self.checks.get(&s.slug())
    }

    /// Takes in the answers that arrived; true when there was one.
    pub fn poll_checks(&mut self) -> bool {
        let got: Vec<_> = self.inbox.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        let any = !got.is_empty();
        for (key, r) in got {
            self.checks.insert(key, match r {
                Ok(rel) => Check::Done(Box::new(rel)),
                Err(e) => Check::Failed(e),
            });
        }
        any
    }

    pub fn checking(&self) -> bool {
        self.checks.values().any(|c| matches!(c, Check::Checking))
    }

    /// Something runs in the background (the window keeps repainting).
    pub fn busy(&self) -> bool {
        self.checking() || self.job.is_some()
    }

    /// What the cached release means for a mod at `installed` (None = not installed).
    pub fn verdict(&self, s: &UpdateSource, id: &str, name: &str, installed: Option<&str>) -> Option<Verdict> {
        match self.check(s)? {
            Check::Done(rel) => Some(update::evaluate(rel, s, id, name, installed)),
            _ => None,
        }
    }
}

/// Everything a mod row needs to draw its update line (computed before the row's closure, which borrows the page state).
#[derive(Default)]
pub struct UpdateView {
    pub source: Option<UpdateSource>,
    /// A native mod with an update source: shown in the Details, never updated by the launcher.
    pub native: bool,
    pub verdict: Option<Verdict>,
    pub checking: bool,
    pub failed: Option<String>,
    /// Why the button is off right now (the game runs, another update is busy).
    pub blocked: Option<&'static str>,
}

impl UpdateView {
    pub fn offer(&self) -> Option<&Offer> {
        match &self.verdict {
            Some(Verdict::Offer(o)) => Some(o),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct UpdateClicks {
    pub update: bool,
    pub notes: bool,
    pub recheck: bool,
}

/// The line under a row's subtitle when an update is waiting: chip, button, "What's new", and the repo the update comes from.
pub fn strip(ui: &mut Ui, v: &UpdateView) -> UpdateClicks {
    let mut c = UpdateClicks::default();
    let (Some(o), Some(src)) = (v.offer(), &v.source) else { return c };
    ui.horizontal_wrapped(|ui| {
        theme::chip(ui, &format!("Update {}", o.to), theme::GREEN);
        c.update = theme::button_enabled(ui, &format!("Update to {}", o.to), v.blocked.is_none());
        c.notes = theme::link(ui, "What's new");
    });
    // on its own line: a wrapped "from" at the end of the button line looks like a mistake
    ui.label(theme::muted(format!("Comes from github.com/{}", src.slug())));
    if let Some(why) = v.blocked {
        ui.label(theme::muted(why));
    }
    c
}

/// The Details lines: where updates come from, what the last check found, and the button to check again.
pub fn details(ui: &mut Ui, v: &UpdateView) -> UpdateClicks {
    let mut c = UpdateClicks::default();
    let Some(src) = &v.source else { return c };
    if v.native {
        ui.label(theme::muted(format!("Native mods run program code, so the launcher does not update them. Its releases are at github.com/{}.", src.slug())));
        return c;
    }
    ui.label(theme::muted(format!("Updates come from github.com/{}.", src.slug())));
    let line = if v.checking {
        "Update check: checking...".to_string()
    } else if let Some(why) = &v.failed {
        format!("Update check: {why}")
    } else {
        match &v.verdict {
            Some(Verdict::UpToDate(ver)) => format!("Update check: up to date (the newest release is {ver})."),
            Some(Verdict::Offer(o)) => format!("Update check: version {} is available.", o.to),
            Some(Verdict::Skipped(why)) => format!("Update check: {why}."),
            None => String::new(),
        }
    };
    if !line.is_empty() {
        ui.label(theme::muted(line));
    }
    c.recheck = theme::button_enabled(ui, "Check for updates", !v.checking);
    c
}

impl App {
    /// Why an update cannot start right now (None = it can).
    fn mods_update_blocked(&self) -> Option<&'static str> {
        if self.read_only() {
            Some("Close the game to update.")
        } else if self.mod_updates.job.is_some() {
            Some("Another mod update is running.")
        } else if self.loader_upd.job_running() {
            Some("The mod loader is being changed. Wait a moment.")
        } else if self.busy() {
            Some("Wait until the setup or backup is finished.")
        } else {
            None
        }
    }

    /// Is the first script mod in `FEATURED` form the launcher can offer (the game knows script mods)?
    fn mods_featured_wanted(&self) -> bool {
        self.info.installed && gating::available(&self.info.local_version, gating::SCRIPT_MODS)
    }

    fn mods_featured_missing(&self, id: &str) -> bool {
        self.mods_ui.store.get(id).is_none() && !self.mods_dir().join(id).exists()
    }

    /// Asks for the releases the page needs (each repo once per launcher start). Called when the Mods page is drawn.
    pub(super) fn mods_update_checks(&mut self) {
        let mut wanted: Vec<UpdateSource> = self.mods_ui.store.entries.iter().filter_map(|e| e.script.as_ref()).filter(|m| m.kind == "script").filter_map(|m| m.update.clone()).collect();
        if self.mods_featured_wanted() {
            wanted.extend(FEATURED.iter().filter(|f| self.mods_featured_missing(f.id)).filter_map(|f| UpdateSource::parse(f.repo)));
        }
        for s in wanted {
            self.mod_updates.ensure_checked(&s);
        }
    }

    pub(super) fn mod_update_view(&self, e: &ModEntry) -> UpdateView {
        let Some(m) = e.script.as_ref() else { return UpdateView::default() };
        let Some(src) = m.update.clone() else { return UpdateView::default() };
        if m.kind != "script" {
            return UpdateView { source: Some(src), native: true, ..Default::default() };
        }
        let verdict = self.mod_updates.verdict(&src, &m.id, &m.name, Some(&m.version));
        let (checking, failed) = match self.mod_updates.check(&src) {
            Some(Check::Checking) => (true, None),
            Some(Check::Failed(ReleaseError::NotFound)) => (false, Some("the mod has no release yet.".to_string())),
            Some(Check::Failed(err)) => (false, Some(err.text())),
            _ => (false, None),
        };
        let blocked = self.mods_update_blocked().filter(|_| matches!(verdict, Some(Verdict::Offer(_))));
        UpdateView { source: Some(src), native: false, verdict, checking, failed, blocked }
    }

    /// The offer for an installed script mod (None = up to date, not checked yet, or not offered).
    pub fn mod_update_offer(&self, id: &str) -> Option<Offer> {
        let e = self.mods_ui.store.get(id)?;
        self.mod_update_view(e).offer().cloned()
    }

    /// Starts the update of installed mod `id` on a worker thread.
    pub fn mods_start_update(&mut self, id: &str) -> Result<(), String> {
        if let Some(why) = self.mods_update_blocked() {
            return Err(why.to_string());
        }
        let e = self.mods_ui.store.get(id).cloned().ok_or("That mod is not installed.")?;
        let offer = self.mod_update_offer(id).ok_or("There is no update for that mod.")?;
        let src = offer.source.release_source(self.mod_updates.releases_dir.as_deref());
        self.mod_updates.job = Some(UpdateJob::start(self.mods_dir(), src, offer, Some(e.dir)));
        Ok(())
    }

    /// Starts the download of a mod from `FEATURED` that is not installed (it is added switched off).
    pub fn mods_start_featured(&mut self, id: &str) -> Result<(), String> {
        if let Some(why) = self.mods_update_blocked() {
            return Err(why.to_string());
        }
        let f = FEATURED.iter().find(|f| f.id == id).ok_or("Unknown mod.")?;
        let src = UpdateSource::parse(f.repo).ok_or("Unknown repo.")?;
        if !self.mods_featured_missing(id) {
            return Err("That mod is already installed.".to_string());
        }
        let Some(Verdict::Offer(offer)) = self.mod_updates.verdict(&src, f.id, f.name, None) else { return Err("There is no release to download yet.".to_string()) };
        let rs = src.release_source(self.mod_updates.releases_dir.as_deref());
        self.mod_updates.job = Some(UpdateJob::start(self.mods_dir(), rs, *offer, None));
        Ok(())
    }

    /// The "Get <mod>" buttons of the toolbar for the featured mods that are not installed and have a release.
    pub(super) fn mods_get_buttons(&mut self, ui: &mut Ui) {
        if !self.mods_featured_wanted() {
            return;
        }
        let blocked = self.mods_update_blocked().is_some();
        for f in FEATURED {
            let Some(src) = UpdateSource::parse(f.repo) else { continue };
            if !self.mods_featured_missing(f.id) || !matches!(self.mod_updates.verdict(&src, f.id, f.name, None), Some(Verdict::Offer(_))) {
                continue;
            }
            if theme::button_enabled(ui, &format!("Get {}", f.name), !blocked) {
                if let Err(why) = self.mods_start_featured(f.id) {
                    self.toast(why, true);
                }
            }
        }
    }

    /// What the buttons of one row asked for.
    pub(super) fn mod_update_actions(&mut self, e: &ModEntry, c: UpdateClicks) {
        if c.update {
            if let Err(why) = self.mods_start_update(&e.id) {
                self.toast(why, true);
            }
        }
        if c.notes {
            if let Some(o) = self.mod_update_offer(&e.id) {
                platform::open_url(&o.page_url);
            }
        }
        if c.recheck {
            if let Some(src) = e.script.as_ref().and_then(|m| m.update.clone()) {
                self.mod_updates.recheck(&src);
            }
        }
    }

    /// The progress card of a running update (above the list, like the import card).
    pub(super) fn mods_update_card(&mut self, ui: &mut Ui) {
        let Some(job) = self.mod_updates.job.as_ref() else { return };
        let (offer, phase) = (job.offer.clone(), job.phase());
        let (done, total) = (job.transfer.done.load(std::sync::atomic::Ordering::Relaxed), job.transfer.total.load(std::sync::atomic::Ordering::Relaxed));
        let cancelling = job.transfer.cancel.load(std::sync::atomic::Ordering::Relaxed);
        let mut cancel = false;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = if offer.from.is_empty() { format!("Adding \"{}\" {}...", offer.name, offer.to) } else { format!("Updating \"{}\" to {}...", offer.name, offer.to) };
            ui.label(theme::bold(title, 16.0));
            if phase == "Downloading" && total > 0 {
                theme::progress(ui, done as f32 / total as f32, 8.0, theme::BLUE);
            } else {
                theme::progress_busy(ui, 8.0, theme::BLUE);
            }
            let size = if phase == "Downloading" && total > 0 { format!("  -  {} of {}", crate::library::size_text(done), crate::library::size_text(total)) } else { String::new() };
            ui.label(theme::muted(format!("{phase}{size}  -  from github.com/{}", offer.source.slug())));
            if cancelling {
                ui.label(theme::muted("Cancelling and cleaning up..."));
            } else {
                cancel = theme::button_enabled(ui, "Cancel", phase != "Installing");
            }
        });
        if cancel {
            job.cancel();
        }
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }

    /// Takes in finished checks and a finished update (called every frame from `mods_poll`).
    pub(super) fn mods_update_poll(&mut self) {
        self.mod_updates.poll_checks();
        let Some(res) = self.mod_updates.job.as_ref().and_then(|j| j.take_result()) else { return };
        let Some(job) = self.mod_updates.job.take() else { return };
        let dir = self.mods_dir();
        let active = self.mods_ui_active_file();
        self.mods_ui.reload(&dir, active.as_deref());
        if self.mods_ui.settings_open.as_deref() == Some(job.offer.id.as_str()) {
            self.mods_ui.panel = PanelState::default();
        }
        match res {
            Ok(d) => self.mods_update_done_toast(&d),
            Err(m) if m == update::CANCELLED => self.toast("Cancelled. Nothing was changed.", false),
            Err(m) => self.toast(format!("\"{}\" was not updated. {m}", job.offer.name), true),
        }
    }

    fn mods_update_done_toast(&mut self, d: &Done) {
        let text = match &d.backup {
            Some(b) => format!("Updated {} from {} to {}. The old version is kept in {}. It is used the next time the game starts.", d.name, d.from, d.to, b.display()),
            None => format!("Added {} {}. It is switched off: switch it on in the list. It is used the next time the game starts.", d.name, d.to),
        };
        self.toast(text, false);
        if let Some(t) = self.toast.as_mut() {
            t.until = Instant::now() + Duration::from_secs(12);
        }
    }
}
