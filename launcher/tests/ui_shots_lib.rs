//! Offscreen snapshots of the library and profile UI (session 5): library with 2 installs, add-install dialog,
//! profile picker open (+ New form), settings with a greyed row, Play tab with the picker. PNGs go to `shots\` next to the crate folder.
use egui::vec2;
use egui_kittest::Harness;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::state_lib::{AddDialog, EditKind};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::{Path, PathBuf};

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_shots2_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn install(root: &Path, version: &str) {
    for (f, t) in [
        ("build/release/rac-rs.exe", "x".to_string()),
        ("Play.cmd", "x".to_string()),
        ("setup.ps1", "x".to_string()),
        ("rc.iso", "disc".to_string()),
        ("VERSION", format!("{version}\n")),
        ("generated/built_version.txt", format!("{version}\n")),
        ("player/pc_options.txt", "version=1\nlighting=enhanced\n".to_string()),
        ("generated/play.cfg", format!("iso={}\n", root.join("rc.iso").display())),
    ] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
}

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn app(name: &str, version: &str) -> App {
    let root = scratch(name);
    install(&root, version);
    let mut a = App::new(root.clone(), Some(root.join("_no_remote").to_string_lossy().to_string()));
    a.covers.dir = Some(root.join("_no_covers"));
    a
}

fn shot(name: &str, a: App) {
    let _g = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).with_pixels_per_point(1.0).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn s24_library_two_installs() {
    let mut a = app("lib", "0.3.5");
    let second = scratch("lib2");
    install(&second, "0.3.1");
    a.library.add_existing(&second).unwrap();
    a.sel = Sel::Library;
    shot("24_library_two_installs", a);
}

#[test]
fn s25_add_install_dialog() {
    let mut a = app("add", "0.3.5");
    a.sel = Sel::Library;
    a.lib_ui.dialog = Some(AddDialog::New { text: r"C:\Users\Player\Documents\RAC-RS\Ratchet & Clank".into() });
    shot("25_new_install_dialog", a);
    let mut b = app("add2", "0.3.5");
    b.sel = Sel::Library;
    b.lib_ui.dialog = Some(AddDialog::Existing { text: r"D:\Games\no such folder".into() });
    shot("26_add_existing_dialog", b);
}

#[test]
fn s27_profile_picker_open() {
    let mut a = app("pick", "0.3.5");
    a.profiles.create("Steam Deck").unwrap();
    a.profile_list = a.profiles.list();
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    a.profile_ui.open = true;
    shot("27_profile_picker_open", a);
    let mut b = app("pick2", "0.3.5");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Settings;
    b.profile_ui.open = true;
    b.profile_begin(EditKind::New);
    shot("28_profile_new_form", b);
}

#[test]
fn s29_settings_greyed_row_and_play_tab() {
    let mut a = app("grey", "0.3.1");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Settings;
    shot("29_settings_greyed_renderer", a);
    let mut b = app("running", "0.3.5");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Settings;
    b.game_external = true;
    shot("30_settings_game_running_readonly", b);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// Stage C shots (session 7): version picker, download card, launcher update banner, pinned update line, saves button, old-version levels.

use egui_kittest::kittest::Queryable;
use rac_rs_launcher::releases::{Asset, Release};
use rac_rs_launcher::selfupdate::{LauncherUpdate, SelfState};
use rac_rs_launcher::ui::state_net::VersionList;
use rac_rs_launcher::update::{UpdateOffer, UpdateState};

/// Like `shot`, with a window size, a pixel scale and a step that may click things first.
fn shot_with(name: &str, a: App, size: egui::Vec2, ppp: f32, act: impl FnOnce(&mut Harness<'static, LauncherApp>)) {
    let _g = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..4 {
        h.step();
    }
    act(&mut h);
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

fn release(tag: &str, date: &str, pre: bool, size_mb: u64) -> Release {
    let ver = tag.trim_start_matches('v');
    Release {
        tag_name: tag.to_string(),
        body: Some(format!("Release {ver}")),
        assets: vec![Asset {
            name: format!("rac-rs-{ver}.zip"),
            size: size_mb * 1024 * 1024,
            browser_download_url: format!("https://example.invalid/rac-rs-{ver}.zip"),
            digest: Some(format!("sha256:{}", "ab".repeat(32))),
        }],
        draft: false,
        prerelease: pre,
        published_at: Some(format!("{date}T18:00:00Z")),
    }
}

fn some_releases() -> Vec<Release> {
    vec![
        release("v0.3.5-rc1", "2026-10-23", true, 6),
        release("v0.3.1", "2026-10-22", false, 7),
        release("v0.3.0", "2026-10-21", false, 7),
        release("v0.2.2", "2026-10-18", false, 5),
        release("v0.2.1", "2026-10-16", false, 5),
    ]
}

fn picker_app(name: &str) -> App {
    let mut a = app(name, "0.3.1");
    a.sel = Sel::Library;
    a.lib_ui.dialog = Some(AddDialog::New { text: r"C:\Users\Player\Documents\RAC-RS\Ratchet & Clank 0.2.2".into() });
    *a.net.versions.lock().unwrap() = VersionList::Ready(some_releases());
    a
}

#[test]
fn s31_version_picker_closed() {
    // closed: the newest is picked, test versions hidden
    shot_with("31_version_picker_closed", picker_app("vp1"), vec2(1100.0, 760.0), 1.0, |_| {});
}

#[test]
fn s32_version_picker_older_picked() {
    // an older version picked, test versions ticked: the note about side-by-side
    let mut b = picker_app("vp2");
    b.net.show_test = true;
    b.net.pick = Some("0.2.2".into());
    shot_with("32_version_picker_older_picked_test_ticked", b, vec2(1100.0, 760.0), 1.0, |_| {});
}

#[test]
fn s33_version_picker_list_open() {
    // open list with test versions
    let mut c = picker_app("vp3");
    c.net.show_test = true;
    shot_with("33_version_picker_list_open", c, vec2(1100.0, 760.0), 1.0, |h| {
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
    });
}

#[test]
fn s34_download_progress_card() {
    use std::sync::atomic::Ordering;
    let mut a = app("dl", "0.3.1");
    a.sel = Sel::Library;
    let dl = rac_rs_launcher::game_dl::GameDl {
        transfer: rac_rs_launcher::releases::Transfer::new(),
        stage: std::sync::Arc::new(std::sync::atomic::AtomicU8::new(rac_rs_launcher::game_dl::STAGE_DOWNLOAD)),
        started: std::time::Instant::now() - std::time::Duration::from_secs(9),
        dest: PathBuf::from(r"C:\Users\Player\Documents\RAC-RS\Ratchet & Clank 0.2.2"),
        version: std::sync::Arc::new(std::sync::Mutex::new("0.2.2".to_string())),
        result: std::sync::Arc::new(std::sync::Mutex::new(None)),
    };
    dl.transfer.total.store(5 * 1024 * 1024 + 300_000, Ordering::Relaxed);
    dl.transfer.done.store(3 * 1024 * 1024 + 100_000, Ordering::Relaxed);
    a.net.game_dl = Some(dl);
    shot_with("34_download_progress_card", a, vec2(1100.0, 700.0), 1.0, |_| {});
    // a failed download
    let mut b = app("dl2", "0.3.1");
    b.sel = Sel::Library;
    b.net.dl_error = Some("There is no internet connection. Check your network and try again.".to_string());
    shot_with("35_download_failed_card", b, vec2(1100.0, 700.0), 1.0, |_| {});
}

fn launcher_update() -> SelfState {
    SelfState::Available(LauncherUpdate {
        version: "0.2.0".into(),
        notes: "- New: pick an older game version when you make a new install.\n- New: Open saves folder.\n- Fixed: the profile list could stay open after a click.".into(),
        asset: Asset { name: "rac-rs-launcher-windows-x64.exe".into(), size: 7 * 1024 * 1024, browser_download_url: "https://example.invalid/x.exe".into(), digest: Some(format!("sha256:{}", "cd".repeat(32))) },
    })
}

#[test]
fn s36_launcher_update_banner() {
    let mut a = app("lb", "0.3.5");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    *a.net.self_state.lock().unwrap() = launcher_update();
    shot_with("36_launcher_update_banner", a, vec2(1100.0, 700.0), 1.0, |_| {});
    // notes open, small window, on the Library page
    let mut b = app("lb2", "0.3.5");
    b.sel = Sel::Library;
    *b.net.self_state.lock().unwrap() = launcher_update();
    b.net.notes_open = true;
    shot_with("37_launcher_update_banner_notes_small_860x580", b, vec2(860.0, 580.0), 1.0, |_| {});
}

/// Sets the update state AFTER the background check of the scratch install has finished (it would overwrite it otherwise).
fn offer_update(a: &mut App, from: &str, to: &str) {
    for _ in 0..200 {
        if a.update_state() != UpdateState::Checking {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    *a.update.lock().unwrap() = UpdateState::Available(UpdateOffer { from: from.into(), to: to.into(), changes: String::new(), finish: false });
}

#[test]
fn s38_pinned_install_update_line() {
    let mut a = app("pin", "0.2.2");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    let id = a.cur_id.clone().unwrap();
    a.library.set_pinned(&id, true);
    offer_update(&mut a, "0.2.2", "0.3.1");
    shot_with("38_pinned_install_update_line", a, vec2(1100.0, 700.0), 1.0, |_| {});
    // narrow window: the sentence and the button must wrap, not run off the card
    let mut b = app("pin2", "0.2.2");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Play;
    let id = b.cur_id.clone().unwrap();
    b.library.set_pinned(&id, true);
    offer_update(&mut b, "0.2.2", "0.3.1");
    shot_with("39_pinned_install_update_line_small_860x580", b, vec2(860.0, 580.0), 1.0, |_| {});
}

#[test]
fn s40_open_saves_folder_buttons() {
    let mut a = app("sv1", "0.3.1");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    shot_with("40_play_tab_open_saves_folder", a, vec2(1100.0, 700.0), 1.0, |_| {});
    let mut b = app("sv2", "0.3.1");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Play;
    shot_with("41_play_tab_open_saves_folder_small_860x580", b, vec2(860.0, 580.0), 1.0, |_| {});
    let mut c = app("sv3", "0.3.1");
    c.sel = Sel::Library;
    shot_with("42_library_card_open_saves_folder", c, vec2(1100.0, 700.0), 1.0, |_| {});
    let mut d = app("sv4", "0.3.1");
    d.sel = Sel::Library;
    shot_with("43_library_card_open_saves_folder_small_860x580", d, vec2(860.0, 580.0), 1.0, |_| {});
}

#[test]
fn s44_settings_levels_greyed_on_old_version() {
    let mut a = app("lv", "0.2.2");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Settings;
    a.options.set("lighting", Some("enhanced"));
    a.options.set("water", Some("classic"));
    shot_with("44_settings_levels_greyed_on_0.2.2", a, vec2(1100.0, 1250.0), 1.0, |_| {});
}
