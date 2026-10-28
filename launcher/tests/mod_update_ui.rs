//! Mod updates on the Mods page: a mod whose manifest names a GitHub repo shows "Update to X" when that repo's latest release is newer, the click
//! downloads, checks and swaps the folder (the old one is kept in `mod_backups`), and nothing the player made (on/off list, settings.json,
//! the mod's save data) is touched. The "GitHub" here is a folder of `latest.json` + zips (`App.mod_updates.releases_dir`, the same switch as
//! `--mod-releases`); no network is used. The pictures at the end go to `shots\` like the other shot tests.
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use rac_rs_launcher::library::Library;
use rac_rs_launcher::mods::update::UpdateSource;
use rac_rs_launcher::releases;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const ARSENAL: &str = include_str!("data/arsenal-mastery.omni-mod.json");
const ID: &str = "arsenal-mastery";

static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_modup_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

fn install(root: &Path, version: &str) {
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("setup.ps1"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("VERSION"), &format!("{version}\n"));
    write(&root.join("generated/built_version.txt"), &format!("{version}\n"));
    write(&root.join("player/pc_options.txt"), "version=1\nlighting=classic\n");
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
}

/// The real Arsenal Mastery manifest at `ver`, with an `update` key.
fn manifest(ver: &str, repo: Option<&str>) -> String {
    let mut v: Value = serde_json::from_str(ARSENAL).unwrap();
    v["version"] = json!(ver);
    if let Some(r) = repo {
        v["update"] = json!({ "github": r });
    }
    serde_json::to_string_pretty(&v).unwrap()
}

/// The zip of a release: the manifest at the top, main.luau and one file only this version has.
fn package(ver: &str, repo: Option<&str>) -> Vec<u8> {
    let mut zip = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut zip);
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("omni-mod.json", o).unwrap();
        w.write_all(manifest(ver, repo).as_bytes()).unwrap();
        w.start_file("main.luau", o).unwrap();
        w.write_all(format!("-- script {ver}\n").as_bytes()).unwrap();
        w.start_file(format!("only_in_{ver}.txt"), o).unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }
    zip.into_inner()
}

/// `<releases>/<owner>/<repo>/` with a latest.json for `ver` (digest right unless `bad_digest`).
fn release(rels: &Path, repo: &str, ver: &str, bad_digest: bool) {
    let zip = package(ver, Some("someone/demo"));
    let name = format!("{ID}-{ver}.zip");
    releases::write_test_release(&rels.join(repo), &format!("v{ver}"), "Fixed things.", &[(name.as_str(), zip.as_slice())], &|_, hex| {
        Some(if bad_digest { format!("sha256:{}", "0".repeat(64)) } else { format!("sha256:{hex}") })
    });
}

struct Env {
    data: PathBuf,
    root: PathBuf,
    rels: PathBuf,
}

/// An install at game `game_version` with whatever `mods` writes; the update checks read `Env.rels`.
fn app_with(name: &str, game_version: &str, mods: impl FnOnce(&Path)) -> (App, Env) {
    let data = scratch(&format!("{name}_data"));
    let root = scratch(&format!("{name}_root"));
    let rels = scratch(&format!("{name}_rels"));
    install(&root, game_version);
    mods(&root);
    let mut lib = Library::load(&data);
    let id = lib.add_existing(&root).unwrap().id;
    lib.save().unwrap();
    let mut app = App::with_library(data.clone(), lib, Some(id), Some(data.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(data.join("_no_covers"));
    app.mod_updates.releases_dir = Some(rels.clone());
    app.reload_game_data();
    app.sel = Sel::Game("rac1");
    app.tab = Tab::Mods;
    (app, Env { data, root, rels })
}

/// Arsenal Mastery 0.3.2 with the update key, plus the player's own files that an update must not touch.
fn old_mod(root: &Path) {
    write(&root.join("mods").join(ID).join("omni-mod.json"), &manifest("0.3.2", Some("someone/demo")));
    write(&root.join("mods").join(ID).join("main.luau"), "-- script 0.3.2\n");
    write(&root.join("mods").join(ID).join("only_in_0.3.2.txt"), "x");
    write(&root.join("mods/settings.json"), r#"{"format":1,"mods":{"arsenal-mastery":{"rebalance":"hard"}}}"#);
    write(&root.join("profiles/default/active.txt"), "# Enabled mods, one id per line, highest priority first.\narsenal-mastery\n");
    write(&root.join("player/omni/slot1.json"), r#"{"xp":{"wrench":1234}}"#);
}

fn harness(app: App) -> Harness<'static, LauncherApp> {
    Harness::builder().with_size(vec2(1100.0, 900.0)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn cleanup(e: &Env) {
    for d in [&e.data, &e.root, &e.rels] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Frames and polling until `until` holds (the checks and the update run on their own threads; the test harness never calls `App::tick`).
fn pump(h: &mut Harness<'static, LauncherApp>, what: &str, until: impl Fn(&Harness<'static, LauncherApp>) -> bool) {
    let t = Instant::now();
    loop {
        h.state_mut().app.mods_poll();
        h.step();
        h.step();
        if until(h) {
            return;
        }
        assert!(t.elapsed() < Duration::from_secs(10), "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn settled(h: &Harness<'static, LauncherApp>) -> bool {
    let a = &h.state().app;
    !a.mod_updates.busy()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn version_in(dir: &Path) -> String {
    let v: Value = serde_json::from_str(&read(&dir.join("omni-mod.json"))).unwrap();
    v["version"].as_str().unwrap().to_string()
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------------------ the row

#[test]
fn a_newer_release_shows_the_chip_the_button_and_where_it_comes_from() {
    let _g = lock();
    let (a, e) = app_with("newer", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.3", false);
    let mut h = harness(a);
    pump(&mut h, "the Update button", |h| h.query_by_label("Update to 0.3.3").is_some());
    // (the green "Update 0.3.3" chip is painted, not a widget: it is in the picture s56)
    h.get_by_label("What's new");
    h.get_by_label_contains("Comes from github.com/someone/demo");
    assert!(h.state().app.mod_update_offer(ID).is_some_and(|o| o.from == "0.3.2" && o.to == "0.3.3" && o.page_url.ends_with("/releases/tag/v0.3.3")));
    cleanup(&e);
}

#[test]
fn an_equal_or_older_release_and_a_missing_release_show_nothing() {
    let _g = lock();
    for (name, rel) in [("equal", Some("0.3.2")), ("older", Some("0.3.1")), ("none", None)] {
        let (a, e) = app_with(name, "0.4.0", old_mod);
        if let Some(v) = rel {
            release(&e.rels, "someone/demo", v, false);
        }
        let mut h = harness(a);
        h.step();
        pump(&mut h, "the check to finish", settled);
        h.step();
        assert!(h.query_by_label_contains("Update to").is_none(), "{name}");
        assert!(h.query_by_label("What's new").is_none(), "{name}");
        assert!(h.state().app.toast.is_none(), "{name}: a check that finds nothing (or fails) must stay quiet");
        cleanup(&e);
    }
}

#[test]
fn the_details_say_what_the_check_found_and_check_again_asks_once_more() {
    let _g = lock();
    let (a, e) = app_with("details", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.2", false);
    let mut h = harness(a);
    pump(&mut h, "the check", settled);
    h.get_by_label("Details").click();
    h.step();
    h.step();
    h.get_by_label_contains("Updates come from github.com/someone/demo");
    h.get_by_label_contains("up to date (the newest release is 0.3.2)");
    // the author publishes 0.3.3; the launcher asks only when told to
    release(&e.rels, "someone/demo", "0.3.3", false);
    h.step();
    assert!(h.query_by_label("Update to 0.3.3").is_none(), "one request per repo per start");
    h.get_by_label("Check for updates").click();
    pump(&mut h, "the second check", |h| h.query_by_label("Update to 0.3.3").is_some());
    cleanup(&e);
}

#[test]
fn a_native_mod_names_its_repo_but_is_never_checked_or_updated() {
    let _g = lock();
    let (a, e) = app_with("native", "0.4.0", |r| {
        write(
            &r.join("mods/native-one/omni-mod.json"),
            r#"{"format":2,"id":"native-one","name":"Native One","version":"1.0.0","author":"A","description":"d","update":{"github":"someone/native"},"games":{"rac1":["pal","ntsc"]},"plugin":{"windows-x64":"n.dll"}}"#,
        );
    });
    release(&e.rels, "someone/native", "1.0.1", false);
    let mut h = harness(a);
    for _ in 0..3 {
        h.step();
    }
    let native = UpdateSource::parse("someone/native").unwrap();
    assert!(h.state().app.mod_updates.check(&native).is_none(), "no request is made for a native mod");
    assert!(h.query_by_label_contains("Update to").is_none());
    h.get_by_label("Details").click();
    h.step();
    h.step();
    h.get_by_label_contains("does not update them");
    h.get_by_label_contains("github.com/someone/native");
    cleanup(&e);
}

// ------------------------------------------------------------------ the update itself

#[test]
fn the_update_button_swaps_the_folder_keeps_the_old_one_and_leaves_the_players_files_alone() {
    let _g = lock();
    let (a, e) = app_with("click", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.3", false);
    let (settings, active, save) = (read(&e.root.join("mods/settings.json")), read(&e.root.join("profiles/default/active.txt")), read(&e.root.join("player/omni/slot1.json")));
    let mut h = harness(a);
    pump(&mut h, "the Update button", |h| h.query_by_label("Update to 0.3.3").is_some());
    h.get_by_label("Update to 0.3.3").click();
    pump(&mut h, "the update to finish", |h| h.state().app.mod_updates.job.is_none() && h.query_by_label("Update to 0.3.3").is_none());
    let m = e.root.join("mods").join(ID);
    assert_eq!(version_in(&m), "0.3.3");
    assert_eq!(read(&m.join("main.luau")), "-- script 0.3.3\n");
    assert!(m.join("only_in_0.3.3.txt").is_file() && !m.join("only_in_0.3.2.txt").exists(), "the folder is replaced as a whole");
    let b = e.root.join("mod_backups/arsenal-mastery-0.3.2");
    assert_eq!(version_in(&b), "0.3.2");
    assert_eq!(read(&b.join("main.luau")), "-- script 0.3.2\n");
    assert!(b.join("only_in_0.3.2.txt").is_file());
    assert!(!e.root.join("mod_update_tmp").exists(), "the staging folder is gone");
    let mut in_mods: Vec<String> = std::fs::read_dir(e.root.join("mods")).unwrap().flatten().map(|d| d.file_name().to_string_lossy().to_string()).collect();
    in_mods.sort();
    assert_eq!(in_mods, vec![ID.to_string(), "settings.json".to_string()], "no backup or staging folder inside mods (the game would load it as a second copy)");
    assert_eq!(read(&e.root.join("mods/settings.json")), settings);
    assert_eq!(read(&e.root.join("profiles/default/active.txt")), active);
    assert_eq!(read(&e.root.join("player/omni/slot1.json")), save);
    h.get_by_label_contains("v0.3.3");
    assert!(h.query_by_label_contains("Update to").is_none() && h.query_by_label("Update 0.3.3").is_none());
    let toast = h.state().app.toast.as_ref().map(|t| (t.text.clone(), t.error));
    assert!(toast.as_ref().is_some_and(|(t, err)| !err && t.contains("Updated Arsenal Mastery from 0.3.2 to 0.3.3") && t.contains("mod_backups")), "{toast:?}");
    cleanup(&e);
}

#[test]
fn a_download_with_the_wrong_checksum_changes_nothing() {
    let _g = lock();
    let (a, e) = app_with("badsum", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.3", true);
    let mut h = harness(a);
    pump(&mut h, "the Update button", |h| h.query_by_label("Update to 0.3.3").is_some());
    h.get_by_label("Update to 0.3.3").click();
    pump(&mut h, "the update to end", |h| h.state().app.mod_updates.job.is_none());
    let m = e.root.join("mods").join(ID);
    assert_eq!(version_in(&m), "0.3.2");
    assert_eq!(read(&m.join("main.luau")), "-- script 0.3.2\n");
    assert!(!e.root.join("mod_backups").join("arsenal-mastery-0.3.2").exists(), "nothing was moved");
    assert!(!e.root.join("mod_update_tmp").exists());
    let toast = h.state().app.toast.as_ref().map(|t| (t.text.clone(), t.error));
    assert!(toast.as_ref().is_some_and(|(t, err)| *err && t.contains("was not updated")), "{toast:?}");
    h.get_by_label("Update to 0.3.3");
    cleanup(&e);
}

#[test]
fn nothing_updates_while_the_game_runs_and_play_waits_for_a_running_update() {
    let _g = lock();
    let (a, e) = app_with("running", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.3", false);
    let mut h = harness(a);
    pump(&mut h, "the Update button", |h| h.query_by_label("Update to 0.3.3").is_some());
    h.state_mut().app.game_external = true;
    h.step();
    h.step();
    assert!(h.get_by_label("Update to 0.3.3").accesskit_node().is_disabled(), "greyed while the game runs");
    assert!(h.state_mut().app.mods_start_update(ID).is_err());
    assert_eq!(version_in(&e.root.join("mods").join(ID)), "0.3.2");
    h.state_mut().app.game_external = false;
    // an update that has started: Play says wait, and nothing else starts
    assert!(h.state_mut().app.mods_start_update(ID).is_ok());
    assert!(h.state_mut().app.mods_start_update(ID).is_err(), "one update at a time");
    let ctx = egui::Context::default();
    h.state_mut().app.play(&ctx);
    assert!(h.state().app.play_error.as_deref().is_some_and(|m| m.contains("mod update")), "{:?}", h.state().app.play_error);
    pump(&mut h, "the update to finish", |h| h.state().app.mod_updates.job.is_none());
    assert_eq!(version_in(&e.root.join("mods").join(ID)), "0.3.3");
    cleanup(&e);
}

// ------------------------------------------------------------------ Get Arsenal Mastery

#[test]
fn get_arsenal_mastery_adds_the_featured_mod_switched_off() {
    let _g = lock();
    let (a, e) = app_with("get", "0.4.0", |r| {
        write(&r.join("profiles/default/active.txt"), "# Enabled mods, one id per line, highest priority first.\n");
    });
    release(&e.rels, "AsobiCodes/arsenal-mastery", "0.3.3", false);
    let mut h = harness(a);
    pump(&mut h, "the Get button", |h| h.query_by_label("Get Arsenal Mastery").is_some());
    h.get_by_label("Get Arsenal Mastery").click();
    pump(&mut h, "the download to finish", |h| h.state().app.mod_updates.job.is_none() && h.query_by_label("Get Arsenal Mastery").is_none());
    let m = e.root.join("mods").join(ID);
    assert_eq!(version_in(&m), "0.3.3");
    assert!(!e.root.join("mod_backups").exists(), "a fresh install backs nothing up");
    assert!(!read(&e.root.join("profiles/default/active.txt")).lines().any(|l| l == ID), "added switched off");
    h.get_by_label("Arsenal Mastery");
    let toast = h.state().app.toast.as_ref().map(|t| (t.text.clone(), t.error));
    assert!(toast.as_ref().is_some_and(|(t, err)| !err && t.contains("switched off")), "{toast:?}");
    cleanup(&e);
}

#[test]
fn no_get_button_when_the_game_is_too_old_or_the_mod_is_there_already() {
    let _g = lock();
    let (a, e) = app_with("old_game", "0.3.1", |_| {});
    release(&e.rels, "AsobiCodes/arsenal-mastery", "0.3.3", false);
    let mut h = harness(a);
    for _ in 0..4 {
        h.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!h.state().app.mod_updates.busy(), "no request while the game cannot use script mods");
    assert!(h.query_by_label_contains("Get Arsenal").is_none());
    cleanup(&e);
    let (a, e) = app_with("have", "0.4.0", old_mod);
    release(&e.rels, "AsobiCodes/arsenal-mastery", "0.3.3", false);
    let mut h = harness(a);
    for _ in 0..4 {
        h.step();
    }
    assert!(h.query_by_label_contains("Get Arsenal").is_none(), "already installed");
    cleanup(&e);
}

// ------------------------------------------------------------------ pictures

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn render(name: &str, h: &mut Harness<'static, LauncherApp>) {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

fn shot_harness(app: App, height: f32) -> Harness<'static, LauncherApp> {
    Harness::builder().with_size(vec2(1100.0, height)).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

#[test]
fn s56_mods_page_with_an_update_waiting_and_its_details() {
    let _g = lock();
    let (a, e) = app_with("shot1", "0.4.0", old_mod);
    release(&e.rels, "someone/demo", "0.3.3", false);
    let mut h = shot_harness(a, 980.0);
    pump(&mut h, "the Update button", |h| h.query_by_label("Update to 0.3.3").is_some());
    h.state_mut().app.mods_ui.selected = Some(ID.to_string());
    pump(&mut h, "the details", |h| h.query_by_label("Check for updates").is_some());
    render("56_mods_update_available_with_details", &mut h);
    cleanup(&e);
}

#[test]
fn s57_mods_page_get_arsenal_mastery_button() {
    let _g = lock();
    let (a, e) = app_with("shot2", "0.4.0", |_| {});
    release(&e.rels, "AsobiCodes/arsenal-mastery", "0.3.3", false);
    let mut h = shot_harness(a, 620.0);
    pump(&mut h, "the Get button", |h| h.query_by_label("Get Arsenal Mastery").is_some());
    render("57_mods_get_arsenal_mastery", &mut h);
    cleanup(&e);
}
