//! Pictures of the v0.5 pages on a game 0.5.0 install: the Achievements tab (7 of 35 unlocked, some counters under way, from
//! `tests/fixtures/ach_sample.json`) and the Settings page with the Fur / Fire / Smoke rows.
//! Run on demand only: `cargo test --test ach_shots -- --ignored`; PNGs go to RAC_LAUNCHER_SHOTS (default `shots\v05`).
use egui::vec2;
use egui_kittest::Harness;
use rac_rs_launcher::games::RAC1_DISCS;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::{Path, PathBuf};

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots/v05")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

fn app(name: &str, tab: Tab) -> App {
    app_at(name, tab, "0.5.0", "version=1\nlighting=enhanced\n")
}

/// A pretend install at game `version` with `options` as its pc_options.txt.
fn app_at(name: &str, tab: Tab, version: &str, options: &str) -> App {
    let root = std::env::temp_dir().join(format!("racl_v05_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("setup.ps1"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("VERSION"), &format!("{version}\n"));
    write(&root.join("generated/built_version.txt"), &format!("{version}\n"));
    write(&root.join("player/pc_options.txt"), options);
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
    let sample = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ach_sample.json")).unwrap();
    write(&root.join("player/achievements.json"), &sample);
    // no network: the update check reads a local folder that says the installed version is the newest
    let remote = root.join("_remote");
    write(&remote.join("VERSION"), &format!("{version}\n"));
    write(&remote.join("CHANGELOG.md"), "# Changelog\n");
    let mut a = App::new(root.clone(), Some(remote.to_string_lossy().to_string()));
    a.covers.dir = Some(root.join("_no_covers"));
    a.info.disc = Some(&RAC1_DISCS[0]);
    a.sel = Sel::Game("rac1");
    a.tab = tab;
    a.reload_game_data();
    a
}

fn shot(name: &str, a: App, h_px: f32) {
    let mut h = Harness::builder().with_size(vec2(1100.0, h_px)).with_pixels_per_point(1.0).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..6 {
        h.step();
    }
    h.render().expect("offscreen render").save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore]
fn v05_pictures() {
    shot("v05_achievements", app("ach", Tab::Achievements), 700.0);
    shot("v05_achievements_tall", app("ach_tall", Tab::Achievements), 2400.0);
    shot("v05_settings_tall", app("settings", Tab::Settings), 8000.0);
}

/// The DLSS preset row (game 0.5.1): live with Upscaler DLSS on a 0.5.1 install, greyed ("needs 0.5.1") on a 0.5.0 one.
#[test]
#[ignore]
fn v051_dlss_preset_pictures() {
    let opts = "version=1\nlighting=enhanced\nupscaler=dlss\ndlss_preset=l\nunderwater=off\n";
    shot("v051_settings_dlss_l", app_at("v051_dlss", Tab::Settings, "0.5.1", opts), 1400.0);
    shot("v051_settings_on_050", app_at("v051_on050", Tab::Settings, "0.5.0", opts), 1400.0);
    shot("v051_settings_fsr", app_at("v051_fsr", Tab::Settings, "0.5.1", "version=1\nlighting=enhanced\nupscaler=fsr\n"), 1400.0);
    shot("v051_settings_tall", app_at("v051_tall", Tab::Settings, "0.5.1", opts), 8000.0);
}
