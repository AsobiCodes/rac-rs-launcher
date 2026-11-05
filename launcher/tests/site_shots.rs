//! Pictures of the launcher for asobicodes.dev (2026-10-26): a game 0.4.0 install with the bundled Arsenal Mastery mod, Play / Settings / Mods.
//! Run on demand only: `cargo test --test site_shots -- --ignored`; PNGs go to RAC_LAUNCHER_SHOTS (default `shots\site`).
use egui::vec2;
use egui_kittest::Harness;
use rac_rs_launcher::games::RAC1_DISCS;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use rac_rs_launcher::update::UpdateState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const BUNDLED_AM: &str = "D:/ratchet/scratch_race/release_tree28/bundled/mods/arsenal-mastery";

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots/site")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &t);
        } else {
            std::fs::copy(e.path(), t).unwrap();
        }
    }
}

fn app(name: &str) -> App {
    let root = std::env::temp_dir().join(format!("racl_site_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("setup.ps1"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("VERSION"), "0.4.0\n");
    write(&root.join("generated/built_version.txt"), "0.4.0\n");
    write(&root.join("player/pc_options.txt"), "version=1\nlighting=enhanced\n");
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
    copy_dir(Path::new(BUNDLED_AM), &root.join("mods/arsenal-mastery"));
    // the update check reads a local copy of the release's VERSION and CHANGELOG instead of GitHub
    let remote = root.join("_remote");
    write(&remote.join("VERSION"), "0.4.0\n");
    std::fs::copy("D:/ratchet/scratch_race/release_tree28/CHANGELOG.md", remote.join("CHANGELOG.md")).unwrap();
    let mut a = App::new(root.clone(), Some(remote.to_string_lossy().to_string()));
    a.covers.dir = Some(root.join("_no_covers"));
    let t = Instant::now();
    while matches!(a.update_state(), UpdateState::Checking) && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(30));
    }
    a.info.disc = Some(&RAC1_DISCS[0]);
    a.sel = Sel::Game("rac1");
    a
}

fn shot(name: &str, a: App) {
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).with_pixels_per_point(1.0).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..4 {
        h.step();
    }
    h.render().expect("offscreen render").save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore]
fn site_pictures() {
    let mut a = app("play");
    a.tab = Tab::Play;
    a.reload_game_data();
    shot("launcher-play", a);
    let mut a = app("settings");
    a.tab = Tab::Settings;
    a.reload_game_data();
    shot("launcher-settings", a);
    let mut a = app("mods");
    a.tab = Tab::Mods;
    a.reload_game_data();
    a.mods_ui.selected = Some("arsenal-mastery".into());
    shot("launcher-mods", a);
}
