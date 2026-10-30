//! "Add to Steam" in the window (Linux; `SteamUi::shown` switches it on for these tests, so they also run on Windows): the Add / Remove button in
//! Settings > Advanced, the tick on the install page, and the note after a first install. The calls into setup.sh are replaced by stand-ins
//! (`steam.run`, `steam.read`); the real call through `sh` is tested in `steam.rs` (Unix).
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use rac_rs_launcher::steam::{Kind, Mode, Outcome, State};
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::state_steam::SteamUi;
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_steam_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A finished install in a scratch folder (nothing to run), no update server.
fn installed(name: &str) -> PathBuf {
    let root = scratch(name);
    let exe = if cfg!(windows) { "build/release/rac-rs.exe" } else { "build/release/rac-rs" };
    for (f, t) in [(exe, "x"), ("Play.cmd", "x"), ("rc.iso", "disc"), ("VERSION", "0.4.2\n"), ("generated/built_version.txt", "0.4.2\n"), ("player/pc_options.txt", "version=1\n")] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
    std::fs::write(root.join("generated/play.cfg"), format!("iso={}\n", root.join("rc.iso").display())).unwrap();
    root
}

fn harness(root: &PathBuf, tab: Tab, steam: SteamUi) -> Harness<'static, LauncherApp> {
    let mut app = App::new(root.clone(), Some(root.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(root.join("_no_covers"));
    app.sel = Sel::Game("rac1");
    app.tab = tab;
    app.steam = steam;
    Harness::builder().with_size(vec2(1100.0, 3600.0)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn toast(h: &Harness<'static, LauncherApp>) -> (String, bool) {
    let t = h.state().app.toast.as_ref().expect("a message");
    (t.text.clone(), t.error)
}

fn shown(run: fn(&Path, Mode) -> Outcome, read: fn(&Path) -> Option<State>) -> SteamUi {
    SteamUi { shown: true, run, read, ..SteamUi::default() }
}

#[test]
fn the_settings_button_adds_then_removes_and_says_what_happened() {
    static HAS: AtomicBool = AtomicBool::new(false);
    static NEXT: AtomicU8 = AtomicU8::new(0); // 0 done, 3 Steam running, 2 Steam not found
    static LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    fn read(_: &Path) -> Option<State> {
        Some(State { found: true, has: HAS.load(Ordering::SeqCst) })
    }
    fn run(_: &Path, m: Mode) -> Outcome {
        LOG.lock().unwrap().push(m.flag());
        match NEXT.load(Ordering::SeqCst) {
            3 => Outcome { kind: Kind::Running, text: "Steam is running. Close Steam first.".to_string() },
            2 => Outcome { kind: Kind::NoSteam, text: "Steam was not found on this PC.".to_string() },
            _ => {
                HAS.store(m == Mode::Add, Ordering::SeqCst);
                Outcome { kind: Kind::Done, text: if m == Mode::Add { "Added it.".to_string() } else { "Removed it.".to_string() } }
            }
        }
    }
    let root = installed("button");
    let mut h = harness(&root, Tab::Settings, shown(run, read));
    h.run();
    h.get_by_label("Add to Steam").click();
    h.run();
    assert_eq!(LOG.lock().unwrap().as_slice(), ["--add-to-steam"]);
    assert_eq!(toast(&h), ("Added it.".to_string(), false));
    h.run();
    assert!(h.query_by_label("Add to Steam").is_none(), "the button turns into Remove while the game is in Steam");
    h.get_by_label("Remove from Steam").click();
    h.run();
    assert_eq!(LOG.lock().unwrap().as_slice(), ["--add-to-steam", "--remove-from-steam"]);
    assert_eq!(toast(&h).0, "Removed it.");
    h.run();
    h.get_by_label("Add to Steam");
    // Steam is open: an error message, nothing changed, the button stays
    NEXT.store(3, Ordering::SeqCst);
    h.get_by_label("Add to Steam").click();
    h.run();
    let (text, error) = toast(&h);
    assert!(error && text.contains("Close Steam first") && text.contains("Nothing was changed"), "{text}");
    h.get_by_label("Add to Steam");
    NEXT.store(2, Ordering::SeqCst);
    h.get_by_label("Add to Steam").click();
    h.run();
    assert_eq!(toast(&h).0, "Steam was not found on this PC.");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_card_is_only_there_when_it_is_switched_on() {
    fn run(_: &Path, _: Mode) -> Outcome {
        panic!("not called");
    }
    fn read(_: &Path) -> Option<State> {
        None
    }
    let root = installed("hidden");
    let mut off = harness(&root, Tab::Settings, SteamUi { shown: false, ..shown(run, read) });
    off.run();
    assert!(off.query_by_label("Add to Steam").is_none() && off.query_by_label("Remove from Steam").is_none());
    // the platform default: Linux shows it, Windows does not (the Windows launcher is unchanged)
    let mut def = harness(&root, Tab::Settings, SteamUi::default());
    def.run();
    assert_eq!(def.query_by_label("Add to Steam").is_some(), cfg!(not(windows)));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_install_page_has_a_tick_that_starts_off() {
    fn run(_: &Path, _: Mode) -> Outcome {
        panic!("not called");
    }
    fn read(_: &Path) -> Option<State> {
        None
    }
    let root = scratch("tick");
    let mut h = harness(&root, Tab::Play, shown(run, read));
    h.run();
    let tick = "Add the game to Steam (for the Steam Deck; Steam must be closed)";
    assert!(!h.state().app.steam.wanted);
    h.get_by_label(tick).click();
    h.run();
    assert!(h.state().app.steam.wanted, "ticked");
    let mut off = harness(&root, Tab::Play, SteamUi { shown: false, ..shown(run, read) });
    off.run();
    assert!(off.query_by_label(tick).is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn after_a_first_install_the_tick_adds_the_game_and_the_note_says_how_it_went() {
    static NEXT: AtomicU8 = AtomicU8::new(0);
    static CALLS: AtomicU8 = AtomicU8::new(0);
    fn read(_: &Path) -> Option<State> {
        Some(State { found: true, has: false })
    }
    fn run(_: &Path, m: Mode) -> Outcome {
        assert_eq!(m, Mode::Add);
        CALLS.fetch_add(1, Ordering::SeqCst);
        match NEXT.load(Ordering::SeqCst) {
            3 => Outcome { kind: Kind::Running, text: "Steam is running. Close Steam first.".to_string() },
            1 => Outcome { kind: Kind::Failed, text: "python3 not found".to_string() },
            _ => Outcome { kind: Kind::Done, text: "Added it.".to_string() },
        }
    }
    let root = installed("after");
    let mut app = App::new(root.clone(), None);
    app.steam = shown(run, read);
    // not ticked: nothing runs, no note
    assert_eq!(app.steam_after_install(), None);
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    app.steam.wanted = true;
    assert_eq!(app.steam_after_install().as_deref(), Some("Added it."));
    NEXT.store(3, Ordering::SeqCst);
    let n = app.steam_after_install().unwrap();
    assert!(n.contains("not in Steam yet") && n.contains("Close Steam first") && n.contains("Add to Steam"), "{n}");
    NEXT.store(1, Ordering::SeqCst);
    assert_eq!(app.steam_after_install().as_deref(), Some("The game could not be added to Steam: python3 not found"));
    // the Windows launcher: never, whatever the tick says
    let mut win = App::new(root.clone(), None);
    win.steam = SteamUi { shown: false, wanted: true, ..shown(run, read) };
    let before = CALLS.load(Ordering::SeqCst);
    assert_eq!(win.steam_after_install(), None);
    assert_eq!(CALLS.load(Ordering::SeqCst), before);
    let _ = std::fs::remove_dir_all(&root);
}
