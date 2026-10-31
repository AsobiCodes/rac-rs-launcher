//! Small screens (issue #1: Steam Deck in Gaming Mode, 1280 x 800 pixels, often with a scale factor above 1): every page must fit the
//! window and the big Play / Install button must be inside it. The window code (`ui::scale::Fit`) lowers the zoom until at least
//! 860 x 580 points fit; these tests give the pages exactly the point size and pixel scale that the window would give them, check where the
//! button is, and save pictures to `shots\deck\after` (and, with RAC_DECK_BEFORE=1, the same pages without the fit to `shots\deck\before`).
//! A text report with the button positions goes to `shots\deck\report.txt`. No window is opened: offscreen wgpu render.
use egui::{vec2, Rect, Vec2};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use rac_rs_launcher::games::RAC1_DISCS;
use rac_rs_launcher::setup_parse::Progress;
use rac_rs_launcher::ui::scale;
use rac_rs_launcher::ui::state::{App, Failure, RunKind, Sel, Tab, View};
use rac_rs_launcher::ui::{theme, LauncherApp};
use rac_rs_launcher::update::UpdateState;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots")));
    let d = d.join("deck");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_deck_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

const CHANGELOG: &str = "# Changelog\n\n## v0.4.0\n- Faster start on slow PCs.\n- New launcher: install, play and mods in one window.\n- Fixed the water flicker in Pokitaru.\n\n## v0.3.1\n- Fixed a freeze at start-up on some AMD cards.\n";

fn installed_root(name: &str, local: &str, remote: &str) -> (PathBuf, String) {
    let root = scratch(name);
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
    write(&root.join("generated/built_version.txt"), &format!("{local}\n"));
    write(&root.join("VERSION"), &format!("{local}\n"));
    write(&root.join("player/pc_options.txt"), "version=1\n");
    let base = root.join("_remote");
    write(&base.join("VERSION"), &format!("{remote}\n"));
    write(&base.join("CHANGELOG.md"), CHANGELOG);
    (root, base.to_string_lossy().to_string())
}

fn make_app(root: PathBuf, base: Option<String>) -> App {
    let mut app = App::new(root.clone(), base);
    app.covers.dir = Some(root.join("_no_covers"));
    let t = Instant::now();
    while matches!(app.update_state(), UpdateState::Checking) && app.info.installed && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(30));
    }
    app.info.disc = Some(&RAC1_DISCS[0]);
    app
}

fn add_mods(root: &Path) {
    let m = root.join("mods");
    for (id, name, author, desc) in [("hd-ratchet", "HD Ratchet and Clank", "Pixel Pete", "Sharper textures for Ratchet, Clank and the HUD."), ("bright-veldin", "Bright Veldin", "Nina", "Warmer colours for the first planet.")] {
        let json = format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"author\":\"{author}\",\"version\":\"1.2\",\"description\":\"{desc}\",\"game\":\"rac1\",\"kind\":\"textures\",\"serials\":[\"SCES-50916\"]}}");
        write(&m.join(id).join("mod.json"), &json);
        write(&m.join(id).join("replacements/0123456789abcdef-00000001.png"), "x");
    }
    write(&m.join("active.txt"), "hd-ratchet\n");
}

fn demo_progress() -> Progress {
    let mut p = Progress::new(false);
    p.step = 2;
    p.progress = 47.0;
    p.status = "Compiling level 7 of 19 (rec_l6)".into();
    p.log = (1..=40).map(|i| format!("   Compiling rec_sh_{i:02} v0.1.0")).collect();
    p
}

/// One page of the launcher in one state; `button` = the text of the big button that must be on screen (if the page has one).
struct Page {
    name: &'static str,
    button: Option<&'static str>,
    make: fn(&str) -> App,
}

fn app_play(n: &str) -> App {
    let (root, base) = installed_root(n, "0.4.0", "0.4.0");
    make_app(root, Some(base))
}
fn app_update(n: &str) -> App {
    let (root, base) = installed_root(n, "0.3.1", "0.4.0");
    make_app(root, Some(base))
}
fn app_update_changes(n: &str) -> App {
    let mut a = app_update(n);
    a.update_changes_open = true;
    a
}
fn app_running(n: &str) -> App {
    let mut a = app_play(n);
    a.game_external = true;
    a
}
fn app_settings(n: &str) -> App {
    let mut a = app_play(n);
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Settings;
    a
}
fn app_mods(n: &str) -> App {
    let (root, base) = installed_root(n, "0.4.0", "0.4.0");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Mods;
    a.reload_game_data();
    a.mods_ui.selected = Some("hd-ratchet".into());
    a
}
fn app_setup(n: &str) -> App {
    let scratch_remote = scratch(&format!("{n}_remote"));
    make_app(scratch(n), Some(scratch_remote.to_string_lossy().to_string()))
}
fn app_installing(n: &str) -> App {
    let mut a = app_setup(n);
    a.view = View::Run;
    a.run_kind = RunKind::Install;
    a.demo_run = Some((demo_progress(), Instant::now() - Duration::from_secs(412)));
    a
}
fn app_failed(n: &str) -> App {
    let mut a = app_setup(n);
    a.view = View::Failed;
    a.failure = Some(Failure {
        message: "The build stopped: there is not enough free disk space on drive D:. About 6 GB are needed.".into(),
        details: "error: no space left on device".into(),
        log: "   Compiling rec_sh_12 v0.1.0\nerror: no space left on device (os error 112)".into(),
        update: false,
    });
    a
}
fn app_library(n: &str) -> App {
    let mut a = app_play(n);
    a.sel = Sel::Library;
    a
}
fn app_launcher(n: &str) -> App {
    let mut a = app_play(n);
    a.sel = Sel::Launcher;
    a
}
fn app_coming_soon(n: &str) -> App {
    let mut a = app_play(n);
    a.sel = Sel::Game("rac2");
    a
}

const PAGES: &[Page] = &[
    Page { name: "play", button: Some("Play"), make: app_play },
    Page { name: "play_update", button: Some("Play"), make: app_update },
    Page { name: "play_update_changes", button: Some("Play"), make: app_update_changes },
    Page { name: "play_running", button: Some("Playing..."), make: app_running },
    Page { name: "settings", button: None, make: app_settings },
    Page { name: "mods", button: None, make: app_mods },
    Page { name: "setup", button: Some("Install"), make: app_setup },
    Page { name: "installing", button: None, make: app_installing },
    Page { name: "failed", button: None, make: app_failed },
    Page { name: "library", button: None, make: app_library },
    Page { name: "launcher", button: None, make: app_launcher },
    Page { name: "coming_soon", button: None, make: app_coming_soon },
];

/// wgpu instances created at the same moment on several test threads crash the Vulkan loader: one render at a time.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What the window code hands to egui for a window of `phys` pixels at system scale `native`: (size in points, pixels per point).
fn window(phys: Vec2, native: f32, fit: bool) -> (Vec2, f32) {
    let zoom = if fit { scale::fit_zoom(phys, native) } else { 1.0 };
    let ppp = native * zoom;
    (phys / ppp, ppp)
}

/// Inside the page area: right of the 236 pt sidebar, inside the 30 x 24 pt margins of the central panel.
fn visible(r: Rect, win: Vec2) -> bool {
    let area = Rect::from_min_max(egui::pos2(236.0, 24.0), egui::pos2(win.x - 30.0, win.y - 24.0)).expand(1.0);
    area.contains_rect(r)
}

struct Shot {
    /// rect of the big button (points), when the page has one and it was found
    button: Option<Rect>,
    win: Vec2,
    ppp: f32,
}

/// Draws one page and saves the picture to `shots\deck\<dir>\<page>_<w>x<h>_s<native>.png`. `dir` = "after" (with the fit) or "before".
fn run_page(p: &Page, phys: Vec2, native: f32, fit: bool, dir: &str, save: bool) -> Shot {
    let app = (p.make)(&format!("{}_{}_{}_{}", p.name, phys.x, native, fit as u8));
    let (win, ppp) = window(phys, native, fit);
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = Harness::builder().with_size(win).with_pixels_per_point(ppp).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app));
    for _ in 0..4 {
        h.step();
    }
    let button = p.button.and_then(|l| h.query_by_label(l).map(|n| n.rect()));
    if save {
        let img = h.render().expect("offscreen render");
        let d = shots_dir().join(dir);
        std::fs::create_dir_all(&d).unwrap();
        img.save(d.join(format!("{}_{}x{}_s{}.png", p.name, phys.x, phys.y, native))).unwrap();
    }
    Shot { button, win, ppp }
}

const DECK: Vec2 = Vec2::new(1280.0, 800.0);
const DECK_SCALES: [f32; 4] = [1.0, 1.25, 1.5, 2.0];
/// Small laptop screens (pixels) at 100 % scale.
const LAPTOPS: [(f32, f32); 4] = [(1366.0, 768.0), (1280.0, 720.0), (1024.0, 600.0), (800.0, 600.0)];

fn line(p: &Page, phys: Vec2, native: f32, s: &Shot) -> (String, bool) {
    let ok = match (p.button, s.button) {
        (Some(_), Some(r)) => visible(r, s.win),
        (Some(_), None) => false,
        (None, _) => true,
    };
    let text = format!(
        "{:<20} {}x{} scale {:<4} -> window {:.0}x{:.0} pt, ppp {:.3}, button {} {}",
        p.name,
        phys.x,
        phys.y,
        native,
        s.win.x,
        s.win.y,
        s.ppp,
        s.button.map(|r| format!("x {:.0}..{:.0} y {:.0}..{:.0}", r.min.x, r.max.x, r.min.y, r.max.y)).unwrap_or_else(|| "-".into()),
        if p.button.is_none() { "(no button)" } else if ok { "VISIBLE" } else { "OUT OF WINDOW" }
    );
    (text, ok)
}

#[test]
fn deck_pages_fit_and_the_play_button_is_on_screen() {
    let mut report = Vec::new();
    let mut bad = Vec::new();
    for native in DECK_SCALES {
        for p in PAGES {
            let s = run_page(p, DECK, native, true, "after", true);
            let (text, ok) = line(p, DECK, native, &s);
            if !ok {
                bad.push(text.clone());
            }
            report.push(format!("after  {text}"));
            // the page area must hold the design minimum: at least 860 x 580 points
            assert!(s.win.x >= scale::MIN_W - 0.5 && s.win.y >= scale::MIN_H - 0.5, "{}: window {:?} points", p.name, s.win);
        }
    }
    for (w, h) in LAPTOPS {
        for p in PAGES.iter().filter(|p| p.button.is_some() && p.name != "play_update_changes") {
            let phys = vec2(w, h);
            let s = run_page(p, phys, 1.0, true, "after", w == 800.0);
            let (text, ok) = line(p, phys, 1.0, &s);
            if !ok {
                bad.push(text.clone());
            }
            report.push(format!("after  {text}"));
        }
    }
    if std::env::var_os("RAC_DECK_BEFORE").is_some() {
        for native in DECK_SCALES {
            for p in PAGES {
                let s = run_page(p, DECK, native, false, "before", true);
                report.push(format!("before {}", line(p, DECK, native, &s).0));
            }
        }
    }
    let _ = std::fs::write(shots_dir().join("report.txt"), report.join("\n") + "\n");
    assert!(bad.is_empty(), "the big button is not on screen:\n{}", bad.join("\n"));
}

#[test]
fn the_fitted_window_is_never_smaller_than_the_design_minimum() {
    for native in [1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0] {
        for phys in [vec2(1280.0, 800.0), vec2(1366.0, 768.0), vec2(1024.0, 600.0), vec2(1920.0, 1080.0), vec2(2560.0, 1600.0)] {
            let (win, _) = window(phys, native, true);
            assert!(win.x >= scale::MIN_W - 0.5 || phys.x / (native * 0.4) < scale::MIN_W, "{phys:?} at {native}: {win:?}");
            assert!(win.y >= scale::MIN_H - 0.5 || phys.y / (native * 0.4) < scale::MIN_H, "{phys:?} at {native}: {win:?}");
        }
    }
}
