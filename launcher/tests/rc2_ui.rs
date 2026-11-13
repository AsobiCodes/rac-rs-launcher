//! R&C 2 in the launcher, without a window: an R&C 2 install (its own exe) opens on its own page with Play and Settings only, the Settings
//! page shows R&C 2's rows (none of R&C 1's lock-on / wrench rows, no preset selector), a click is written to its pc_options.txt, and
//! PNGs of the pages go to RAC_LAUNCHER_SHOTS (default `..\shots_rc2`).
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::PathBuf;

/// wgpu instances created at the same moment on several test threads crash the Vulkan loader (same lock as tests/ui_shots.rs): one test at a time.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn rc2_install(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("racl_rc2_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for (f, t) in [
        ("build/release/rac2-play.exe", "x"),
        ("Play.cmd", "x"),
        ("setup.ps1", "x"),
        ("rc.iso", "disc"),
        ("VERSION", "0.1.0\n"),
        ("generated/built_version.txt", "0.1.0\n"),
        ("player/pc_options.txt", "version=1\nlighting=enhanced\nkbm=x:y\n"),
    ] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
    std::fs::write(root.join("generated/play.cfg"), format!("iso={}\n", root.join("rc.iso").display())).unwrap();
    root
}

fn harness(root: &PathBuf, tab: Tab, h: f32) -> Harness<'static, LauncherApp> {
    let mut app = App::new(root.clone(), Some(root.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(root.join("_no_covers"));
    app.sel = Sel::Game("rac2");
    app.tab = tab;
    Harness::builder().with_size(vec2(1100.0, h)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn shot(h: &mut Harness<'static, LauncherApp>, name: &str) {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots_rc2")));
    let _ = std::fs::create_dir_all(&d);
    h.run();
    h.render().expect("offscreen render").save(d.join(name)).unwrap();
}

#[test]
fn an_rc2_folder_opens_as_rc2_with_play_and_settings_only() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = rc2_install("play");
    let mut h = harness(&root, Tab::Mods, 760.0); // a saved Mods tab falls back to Play
    h.run();
    {
        let app = &h.state().app;
        assert!(app.info.installed && app.is_rac2());
        assert_eq!(app.tab, Tab::Play);
        assert!(app.info.local_version == "0.1.0");
    }
    // the Mods and Achievements tabs are there, greyed out (R&C 2 has neither yet); a click on one changes nothing
    assert!(h.query_by_label("Page: Mods").is_some() && h.query_by_label("Page: Achievements").is_some());
    h.get_by_label("Page: Mods").click();
    h.run();
    assert_eq!(h.state().app.tab, Tab::Play);
    shot(&mut h, "rc2_play.png");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rc2_settings_show_rc2_rows_and_write_pc_options() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let root = rc2_install("settings");
    let mut h = harness(&root, Tab::Settings, 7000.0);
    h.run();
    assert!(h.unnamed_widgets().is_empty(), "{:?}", h.unnamed_widgets());
    for shown in ["Subtitles", "Key names in prompts", "Strafe aiming", "Volumetric fog", "Stick dead zone"] {
        assert!(h.query_by_label(shown).is_some(), "{shown} is missing");
    }
    // "Buy all ammo": R&C 2 ignores the line (its vendor always has the row), so the page has no switch for it
    for hidden in ["Lock-on strength", "Wrench throw", "Supporter drawings", "Auto-equip gadgets", "Graphics quality", "Buy all ammo"] {
        assert!(h.query_by_label(hidden).is_none(), "{hidden} must not be on R&C 2's page");
    }
    // a click is written at once; nothing is greyed by an R&C 1 version
    h.get_by_label("Subtitles").click();
    h.run();
    let text = std::fs::read_to_string(root.join("profiles/default/pc_options.txt")).unwrap_or_default();
    assert!(text.contains("kbm=x:y"), "bindings stay: {text}");
    shot(&mut h, "rc2_settings.png");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn rc2_install_page_and_new_install_form_shots() {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // an unpacked R&C 2 tree that is not set up yet: the pick-your-disc page names R&C 2 and its PAL disc
    let root = std::env::temp_dir().join(format!("racl_rc2_inst_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("VERSION"), "0.1.0\n").unwrap();
    std::fs::write(root.join("setup.ps1"), "param([string]$Game = 'rac2')").unwrap();
    let mut h = harness(&root, Tab::Play, 760.0);
    h.run();
    assert!(h.state().app.is_rac2() && !h.state().app.info.installed);
    shot(&mut h, "rc2_install.png");
    // Library > New install with the game choice on R&C 2 (no network: a missing test folder = "not released yet")
    h.state_mut().app.net.game_src = rac_rs_launcher::releases::ReleaseSource::Folder(root.join("_none"));
    h.state_mut().app.set_new_game("rac2");
    h.state_mut().app.sel = Sel::Library;
    h.state_mut().app.lib_ui.dialog = Some(rac_rs_launcher::ui::state_lib::AddDialog::New { text: "D:/Games/Ratchet & Clank 2".into() });
    h.run();
    std::thread::sleep(std::time::Duration::from_millis(500));
    shot(&mut h, "rc2_new_install.png");
    let _ = std::fs::remove_dir_all(&root);
}
