//! Reads the output of `setup.ps1 -Console` exactly as the old setup window did (Feed-Line, Update-Progress, Get-ErrorText, Get-DetailsText):
//! step markers `== (a)` ... `== (e)` and `== done`, `level N done`, cargo's `Compiling` lines, `expect-compiles: N`, `ERROR:` lines.
use std::time::{Duration, Instant};

pub const STEP_NAMES_INSTALL: [&str; 5] = ["Download tools", "Check disc", "Prepare game code", "Build game", "Finish"];
pub const STEP_NAMES_UPDATE: [&str; 5] = ["Download update", "Check tools", "Prepare game code", "Build game", "Finish"];
/// "Compiling" lines of a complete first build (measured); the bar never goes above 98% for the build.
pub const EXPECTED_COMPILES: u32 = 563;
const MAX_LOG_LINES: usize = 60_000;

#[derive(Clone, Debug)]
pub struct Progress {
    pub update: bool,
    pub step: usize,
    pub levels: u32,
    pub compiled: u32,
    pub expect: u32,
    pub tools_sub: f64,
    pub vu: bool,
    pub finished: bool,
    pub progress: f64,
    pub last_error: String,
    pub status: String,
    /// `desktop shortcut created: <path>` of the worker.
    pub shortcut: Option<String>,
    pub build_t0: Option<Instant>,
    pub log: Vec<String>,
    /// Exit code of the worker once it ended (3 = the update could not be downloaded).
    pub exit_code: Option<i32>,
}

impl Progress {
    pub fn new(update: bool) -> Progress {
        Progress {
            update,
            step: 0,
            levels: 0,
            compiled: 0,
            expect: 0,
            tools_sub: 0.0,
            vu: false,
            finished: false,
            progress: 0.0,
            last_error: String::new(),
            status: String::new(),
            shortcut: None,
            build_t0: None,
            log: Vec::new(),
            exit_code: None,
        }
    }

    pub fn step_names(&self) -> &'static [&'static str; 5] {
        if self.update {
            &STEP_NAMES_UPDATE
        } else {
            &STEP_NAMES_INSTALL
        }
    }

    pub fn feed_line(&mut self, line: &str) {
        if self.log.len() < MAX_LOG_LINES {
            self.log.push(line.to_string());
        }
        let t = line.trim();
        if !t.is_empty() && !t.starts_with("==") {
            self.status = t.chars().take(100).collect();
        }
        if let Some(k) = step_marker(line) {
            let (idx, status) = match k {
                "u" => (0, "Downloading and applying the update ..."),
                "a" => (if self.update { 1 } else { 0 }, "Getting the tools ready ..."),
                "b" => (1, "Checking the disc image ..."),
                "c" => (2, "Making the game code from your disc ..."),
                "c2" => (2, "Making the graphics code from your disc ..."),
                "d" => (3, "Building the game (the long part) ..."),
                _ => (4, "Finishing ..."),
            };
            if k == "c2" {
                self.vu = true;
            }
            self.status = status.to_string();
            self.step = idx;
            if k == "d" && self.build_t0.is_none() {
                self.build_t0 = Some(Instant::now());
            }
        } else if let Some(n) = line.strip_prefix("expect-compiles: ") {
            if let Some(n) = leading_number(n) {
                self.expect = n;
            }
        } else if line.starts_with("== done") {
            self.finished = true;
            self.step = 4;
        } else if is_level_done(line) {
            self.levels += 1;
        } else if line.starts_with(|c: char| c.is_whitespace()) && line.trim_start().starts_with("Compiling ") {
            self.compiled += 1;
        } else if line.contains("downloading Rust") {
            self.tools_sub = 0.15;
        } else if line.contains("downloading Python") {
            self.tools_sub = 0.6;
        } else if line.contains("downloading the GNU assembler") || line.contains("downloading the C linker") {
            // Windows: the GNU assembler; Linux: the C linker (zig)
            self.tools_sub = 0.8;
        } else if let Some(p) = line.split("desktop shortcut created: ").nth(1) {
            self.shortcut = Some(p.trim().to_string());
        }
        if let Some(e) = line.strip_prefix("ERROR: ") {
            self.last_error = e.to_string();
        } else if self.last_error.is_empty() {
            if let Some(e) = cargo_error(line) {
                self.last_error = e;
            }
        }
    }

    /// The bar, 0..=100: the stages are weighted as in the old window; during the build it follows cargo's "Compiling" count
    /// (an update has no count to trust at first: it also creeps by time); never above 98 before the end.
    pub fn update_progress(&mut self, now: Instant) -> u32 {
        let v = match self.step {
            0 => 1.0 + 7.0 * self.tools_sub,
            1 => 9.0,
            2 => {
                if self.vu {
                    29.0
                } else {
                    10.0 + 18.0 * (self.levels as f64 / 19.0).min(1.0)
                }
            }
            3 => {
                let exp = if self.expect > 0 { self.expect } else { EXPECTED_COMPILES } as f64;
                // cargo prints "Compiling" when a crate STARTS
                let done = if self.update { self.compiled.saturating_sub(1) } else { self.compiled } as f64;
                let mut v = 30.0 + 68.0 * (done / exp).min(1.0);
                if self.update {
                    if let Some(t0) = self.build_t0 {
                        let s = now.saturating_duration_since(t0).as_secs_f64();
                        v = v.max(30.0 + 60.0 * (1.0 - (-s / 150.0).exp()));
                    }
                }
                v
            }
            _ => 99.0,
        };
        let v = if self.finished { 100.0 } else { v };
        if v > self.progress {
            self.progress = v;
        }
        self.progress.floor().min(100.0) as u32
    }

    fn step_name(&self) -> &'static str {
        self.step_names()[self.step.min(4)]
    }

    /// Get-ErrorText: what the player reads when the worker failed.
    pub fn error_text(&self) -> String {
        self.error_text_for(!cfg!(windows))
    }

    /// `error_text` for either system (tests check both on any host). `linux` = false keeps the exact Windows texts: the Linux
    /// fix hints and the extra network patterns are not used there.
    pub fn error_text_for(&self, linux: bool) -> String {
        if self.update && self.exit_code == Some(3) {
            return "The update could not be downloaded.\n\nPlease check your internet connection and click Try again. Nothing on your PC was changed.".to_string();
        }
        let all = self.log.join("\n");
        let no_compile: String = self.log.iter().filter(|l| !l.trim_start().starts_with("Compiling ")).cloned().collect::<Vec<_>>().join("\n").to_lowercase();
        let net_text = [
            "could not download", "failed to download", "couldn't resolve", "could not resolve", "resolve host", "timed out", "unable to connect",
            "spurious network", "network failure", "failed to get ",
        ]
        .iter()
        .any(|p| no_compile.contains(p))
            || (no_compile.contains("download of ") && no_compile.contains(" failed"))
            // curl / the system resolver, as seen on Linux only (Windows keeps its old list)
            || (linux
                && ["failed to connect", "connection refused", "connection reset", "name or service not known", "temporary failure in name resolution", "network is unreachable"]
                    .iter()
                    .any(|p| no_compile.contains(p)));
        // Windows: step 0 is only downloads, so any failure there is a network one. Linux: setup.sh's step (a) also runs the pre-checks
        // (folder cannot run programs, missing libasound/libudev, no cargo/python3/cc, Rust too old), so there an ERROR line that
        // is not a network text is shown as it is; "check your internet connection" stays for network texts and for a bare step-0 stop.
        let net = if linux { net_text || (self.step == 0 && self.last_error.is_empty()) } else { self.step == 0 || net_text };
        let lall = all.to_lowercase();
        let disk = ["no space left", "not enough space", "os error 112", "there is not enough space"].iter().any(|p| lall.contains(p));
        let mut m = format!("The step '{}' did not finish.", self.step_name());
        if self.step == 1 {
            m = "The disc image was not accepted.\n\nIt must be Ratchet & Clank PAL / Europe v1.00 or USA v1.00 as one .iso file. Click Try again and pick the right file.".to_string();
        } else if disk {
            m += &format!("\n\nThere may not be enough free disk space (about {} GB are needed). Free some space and click Try again.", crate::library::NEEDED_GB);
        } else if let Some(h) = system_hint(&self.log, linux, net_text) {
            m += &format!("\n\n{h}");
            // the hint never hides what the setup really said
            if !self.last_error.is_empty() {
                m += &format!("\n\nThe last error line was: {}", self.last_error);
            }
            m += "\n\nThen click Try again. If it keeps failing, click Copy details and share them when you ask for help.";
        } else if net {
            m += "\n\nPlease check your internet connection, then click Try again. Setup continues where it stopped.";
        } else {
            if !self.last_error.is_empty() {
                m += &format!("\n\n{}", self.last_error);
            }
            m += "\n\nClick Try again. If it keeps failing, click Copy details and share them when you ask for help.";
        }
        if self.update {
            m += "\n\nYour old version is still installed. Click Play old version to keep playing it.";
        }
        m
    }

    /// Get-DetailsText: for "Copy details". `system` is the operating system text (`platform::os_text`).
    pub fn details_text(&self, system: &str) -> String {
        let tail: Vec<&String> = self.log.iter().rev().take(200).collect::<Vec<_>>().into_iter().rev().collect();
        let log = tail.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let label = if cfg!(windows) { "Windows" } else { "System" };
        format!("Ratchet & Clank (rac-rs) setup - problem report\nStep: {}\nError: {}\n{}: {}\n\nLog (last 200 lines):\n{}", self.step_name(), self.last_error, label, system, log)
    }

    pub fn elapsed_text(since: Instant, now: Instant) -> String {
        format_elapsed(now.saturating_duration_since(since))
    }
}

pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// Lines cargo (and setup.sh's own progress lines) print about work that went well: they name crates (`Downloaded alsa-sys`) and must never
/// count as evidence for a problem.
fn is_status_line(l: &str) -> bool {
    let t = l.trim_start().to_lowercase();
    ["compiling ", "downloaded ", "downloading ", "fresh ", "checking ", "locking ", "adding ", "updating ", "compiled ", "building ", "finished "].iter().any(|p| t.starts_with(p))
}

/// `zig` as a word of its own (`zig:`, `zig cc`, `zig-linux-...`), not inside `ziglang` or `zigzag`.
fn mentions_zig(l: &str) -> bool {
    l.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == "zig")
}

/// Linux problems that have a known fix a player can do. `linux` = false gives no hint at all (the Windows failure texts stay as they were).
/// `net` = the log looks like a network failure: that text wins over the zig hint (a DNS error names `ziglang.org`).
/// Only lines that are not cargo status lines count, and a library is blamed only when ONE line says it is missing (a pkg-config failure,
/// `cannot find -lasound`, `libasound.so: cannot open shared object file`), so an unrelated error after the crate downloads shows its own text.
fn system_hint(log: &[String], linux: bool, net: bool) -> Option<String> {
    if !linux {
        return None;
    }
    let lines: Vec<String> = log.iter().filter(|l| !is_status_line(l)).map(|l| l.to_lowercase()).collect();
    let any = |f: &dyn Fn(&str) -> bool| lines.iter().any(|l| f(l));
    // setup.sh's own `ERROR: ...` lines already name what is wrong (and the install command for this distribution), and some of them
    // list package names in passing (`no C compiler found (cc/gcc): install gcc, pkg-config and the libasound2-dev ...`): they never blame a library
    let libs: Vec<String> = log.iter().filter(|l| !is_status_line(l) && !l.starts_with("ERROR: ")).map(|l| l.to_lowercase()).collect();
    let any_lib = |f: &dyn Fn(&str) -> bool| libs.iter().any(|l| f(l));
    let miss = |l: &str| ["was not found", "pkg-config", "cannot find", "no such file", "cannot open shared object"].iter().any(|p| l.contains(p));
    let pkgconfig = |l: &str| l.contains("pkg-config") || l.contains("was not found") || l.contains(".pc");
    if any_lib(&|l| l.contains("-lasound") || l.contains("alsa.pc") || l.contains("asoundlib.h") || ((l.contains("alsa") || l.contains("libasound")) && miss(l))) {
        let dev = if any_lib(&|l| (l.contains("alsa") || l.contains("libasound")) && pkgconfig(l)) { " (the development package: libasound2-dev, alsa-lib-devel or alsa-lib)" } else { "" };
        return Some(format!("The sound library ALSA is missing{dev}. Install it with your package manager (Debian/Ubuntu: libasound2, Fedora: alsa-lib, Arch: alsa-lib)."));
    }
    if any_lib(&|l| l.contains("-ludev") || l.contains("libudev.pc") || ((l.contains("udev")) && miss(l))) {
        let dev = if any_lib(&|l| l.contains("udev") && pkgconfig(l)) { " (the development package: libudev-dev, systemd-devel or systemd-libs)" } else { "" };
        return Some(format!("The device library libudev is missing{dev}. Install it with your package manager (Debian/Ubuntu: libudev1, Fedora: systemd-libs, Arch: systemd-libs)."));
    }
    if any(&|l| l.contains("linker `cc` not found") || l.contains("linker 'cc' not found") || l.contains("linker cc not found")) {
        return Some("No C compiler (gcc) was found. Switch off \"Use my system's Rust and Python\" in Settings > Advanced so the launcher uses its own tools, or install gcc (Debian/Ubuntu: build-essential, Fedora: gcc, Arch: base-devel).".to_string());
    }
    if any(&|l| l.contains("python3: not found") || l.contains("python3: command not found") || l.contains("python3 was not found")) {
        return Some("Python 3 was not found. Switch off \"Use my system's Rust and Python\" in Settings > Advanced so the launcher uses its own, or install python3 with your package manager.".to_string());
    }
    if !net && any(&|l| !l.contains("download") && mentions_zig(l) && (l.contains("error") || l.contains("failed") || l.contains("could not"))) {
        return Some("The C linker the launcher downloads (zig) did not work on this system. Switch on \"Use my system's Rust and Python\" in Settings > Advanced and make sure gcc is installed (Debian/Ubuntu: build-essential), or check that your download is not blocked.".to_string());
    }
    None
}

fn leading_number(s: &str) -> Option<u32> {
    let d: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    d.parse().ok()
}

/// `== (u)`, `== (a)` ... `== (e)`, `== (c2)`.
fn step_marker(line: &str) -> Option<&str> {
    let r = line.strip_prefix("== (")?;
    let end = r.find(')')?;
    let k = &r[..end];
    matches!(k, "u" | "a" | "b" | "c" | "c2" | "d" | "e").then_some(k)
}

/// `^\s*level \d+ done`
fn is_level_done(line: &str) -> bool {
    let Some(r) = line.trim_start().strip_prefix("level ") else { return false };
    let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
    !digits.is_empty() && r[digits.len()..].starts_with(" done")
}

/// `^error(\[E\d+\])?: (.*)$` (a compiler error)
fn cargo_error(line: &str) -> Option<String> {
    let r = line.strip_prefix("error")?;
    let r = if let Some(b) = r.strip_prefix("[E") {
        let end = b.find(']')?;
        if !b[..end].bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        &b[end + 1..]
    } else {
        r
    };
    r.strip_prefix(": ").map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(p: &mut Progress, text: &str) {
        for l in text.lines() {
            p.feed_line(l);
        }
    }

    #[test]
    fn markers_levels_and_compiles_of_a_real_setup_run() {
        // the console output of a full setup.ps1 run (RAC_TEST_SETUP_LOG), skipped when not set
        let Some(path) = std::env::var_os("RAC_TEST_SETUP_LOG") else { return };
        let Ok(bytes) = std::fs::read(path) else { return };
        let text = String::from_utf8_lossy(&bytes).to_string();
        let mut p = Progress::new(false);
        let mut steps = vec![];
        for l in text.lines() {
            p.feed_line(l);
            if steps.last() != Some(&p.step) {
                steps.push(p.step);
            }
        }
        assert_eq!(steps, vec![0, 1, 2, 3, 4], "tools, disc, code (+VU), build, finish");
        assert_eq!(p.levels, 19, "19 levels");
        // (setup_full.log predates the (c2) VU step, so p.vu is not asserted)
        assert!(p.finished);
        assert_eq!(p.compiled, text.lines().filter(|l| l.trim_start().starts_with("Compiling ") && l.starts_with(' ')).count() as u32);
        assert!(p.compiled > 400, "{}", p.compiled);
        assert!(p.update_progress(Instant::now()) == 100);
        assert_eq!(p.step_names()[3], "Build game");
    }

    #[test]
    fn bar_follows_the_build_and_stops_at_98() {
        let mut p = Progress::new(false);
        feed_all(&mut p, "== (a) tools\n== (b) disc image\n== (c) generating game code\n== (c2) VU\n== (d) building the game");
        assert_eq!(p.step, 3);
        let a = p.update_progress(Instant::now());
        assert_eq!(a, 30);
        for i in 0..237 {
            p.feed_line(&format!("   Compiling crate-{i} v0.1.0"));
        }
        let b = p.update_progress(Instant::now());
        assert!(b > 30 && b < 98, "{b}");
        for i in 0..2000 {
            p.feed_line(&format!("   Compiling crate-{i} v0.1.0"));
        }
        assert_eq!(p.update_progress(Instant::now()), 98);
        p.feed_line("== done");
        assert_eq!(p.update_progress(Instant::now()), 100);
    }

    #[test]
    fn update_mode_steps_and_expect_compiles() {
        let mut p = Progress::new(true);
        feed_all(&mut p, "== (u) updating to v0.4.0\n  downloading v0.4.0 ...\n  4 files updated, 0 removed\nexpect-compiles: 4\n== (a) tools");
        assert_eq!(p.step, 1);
        assert_eq!(p.expect, 4);
        assert_eq!(p.step_names()[0], "Download update");
        p.feed_line("== (d) building the game");
        p.feed_line("   Compiling rac-rs v0.4.0");
        // the first "Compiling" of an update is not counted as done
        let v = p.update_progress(Instant::now());
        assert!((30..=40).contains(&v), "{v}");
    }

    #[test]
    fn errors_shortcut_and_status() {
        let mut p = Progress::new(false);
        p.feed_line("error[E0425]: cannot find value `x`");
        p.feed_line("error: second one");
        assert_eq!(p.last_error, "cannot find value `x`");
        p.feed_line("ERROR: build failed badly");
        assert_eq!(p.last_error, "build failed badly");
        p.feed_line("  desktop shortcut created: C:\\Users\\x\\Desktop\\Ratchet & Clank (rac-rs).lnk");
        assert_eq!(p.shortcut.as_deref(), Some("C:\\Users\\x\\Desktop\\Ratchet & Clank (rac-rs).lnk"));
        assert!(p.status.starts_with("desktop shortcut created"));
    }

    #[test]
    fn error_texts() {
        let mut p = Progress::new(false);
        p.feed_line("== (b) disc image");
        assert!(p.error_text().starts_with("The disc image was not accepted."));
        let mut p = Progress::new(false);
        p.feed_line("== (d) building");
        p.feed_line("error: failed to get `foo` as a dependency");
        assert!(p.error_text().contains("internet connection"));
        let mut p = Progress::new(false);
        p.feed_line("== (c) code");
        p.feed_line("OSError: [Errno 28] No space left on device");
        assert!(p.error_text().contains("free disk space"));
        let mut p = Progress::new(true);
        p.exit_code = Some(3);
        assert!(p.error_text().starts_with("The update could not be downloaded."));
        let mut p = Progress::new(true);
        p.feed_line("== (d) building");
        p.feed_line("ERROR: weird");
        let t = p.error_text();
        assert!(t.contains("weird") && t.contains("Play old version"), "{t}");
        let d = p.details_text("Windows 11");
        assert!(d.contains("Step: Build game") && d.contains(&format!("{}: Windows 11", if cfg!(windows) { "Windows" } else { "System" })), "{d}");
        let mut p = Progress::new(false);
        p.feed_line("== (c) code");
        p.feed_line("OSError: [Errno 28] No space left on device");
        assert!(p.error_text().contains(&format!("about {} GB", crate::library::NEEDED_GB)));
    }

    /// The console output a setup.sh run prints (shape of CONTRACT section 2): tools incl. the C linker, disc, code, build, finish.
    const LINUX_LOG: &str = "== (a) tools\n  downloading Rust 1.99.0 ...\n  downloading Python 3.12 ...\n  downloading the C linker (zig 0.14.1) ...\n\
== (b) disc image\n  disc ok: PAL\n== (c) generating game code\n  level 1 done\n  level 2 done\n== (c2) VU\nexpect-compiles: 3\n== (d) building the game\n   Compiling a v1\n   Compiling b v1\n   Compiling c v1\n\
== (e) play.cfg\n  desktop shortcut created: /home/p/.local/share/applications/rac-rs-launcher.desktop\n== done\n";

    #[test]
    fn a_linux_setup_log_drives_the_bar() {
        let mut p = Progress::new(false);
        let mut steps = vec![];
        let mut bar = vec![];
        for l in LINUX_LOG.lines() {
            p.feed_line(l);
            if steps.last() != Some(&p.step) {
                steps.push(p.step);
            }
            bar.push(p.update_progress(Instant::now()));
            if l.contains("downloading the C linker") {
                assert_eq!(p.tools_sub, 0.8, "the C linker line moves the tools part");
                assert_eq!(bar.last(), Some(&6), "1 + 7 * 0.8 = 6.6");
            }
        }
        assert_eq!(steps, vec![0, 1, 2, 3, 4]);
        assert!(p.finished && p.levels == 2 && p.compiled == 3 && p.expect == 3);
        assert_eq!(*bar.last().unwrap(), 100);
        assert!(bar.windows(2).all(|w| w[0] <= w[1]), "the bar never goes back: {bar:?}");
        assert_eq!(p.shortcut.as_deref(), Some("/home/p/.local/share/applications/rac-rs-launcher.desktop"));
        // a real setup.sh log, when the path is given
        if let Some(path) = std::env::var_os("RAC_TEST_SETUP_LOG_LINUX") {
            if let Ok(bytes) = std::fs::read(path) {
                let mut r = Progress::new(false);
                for l in String::from_utf8_lossy(&bytes).lines() {
                    r.feed_line(l);
                }
                assert!(r.finished && r.update_progress(Instant::now()) == 100);
            }
        }
    }

    #[test]
    fn linux_failures_get_a_fix_to_do() {
        let mk = |step_line: &str, lines: &[&str]| {
            let mut p = Progress::new(false);
            p.feed_line(step_line);
            for l in lines {
                p.feed_line(l);
            }
            p.error_text_for(true)
        };
        let t = mk("== (d) building", &["   Compiling alsa-sys v0.3.1", "error: failed to run custom build command for `alsa-sys v0.3.1`", "  The system library `alsa` required by crate `alsa-sys` was not found.", "  Could not run `pkg-config --libs alsa`"]);
        assert!(t.contains("ALSA") && t.contains("libasound2-dev") && t.contains("Try again"), "{t}");
        // the hint still shows the real last error line
        assert!(t.contains("The last error line was: failed to run custom build command"), "{t}");
        let t = mk("== (d) building", &["error: linking with `cc` failed", "  = note: /usr/bin/ld: cannot find -ludev: No such file or directory"]);
        assert!(t.contains("libudev") && !t.contains("internet"), "{t}");
        let t = mk("== (d) building", &["error: linker `cc` not found", "  = note: No such file or directory (os error 2)"]);
        assert!(t.contains("Settings > Advanced") && t.contains("gcc"), "{t}");
        let t = mk("== (c) code", &["setup.sh: line 40: python3: not found"]);
        assert!(t.contains("Python 3") && t.contains("Settings > Advanced"), "{t}");
        let t = mk("== (a) tools", &["  downloading the C linker (zig 0.14.1) ...", "zig: error: unable to find library", "ERROR: zig cc did not work"]);
        assert!(t.contains("zig") && t.contains("Use my system"), "{t}");
        // a plain network problem in the same step still says internet, also when zig is being downloaded
        let t = mk("== (a) tools", &["  downloading the C linker (zig 0.14.1) ...", "ERROR: could not download zig-linux-x86_64-0.14.1.tar.xz"]);
        assert!(t.contains("internet connection"), "{t}");
        // crate names alone (Compiling lines) never trigger a hint
        let t = mk("== (d) building", &["   Compiling alsa-sys v0.3.1", "   Compiling libudev-sys v0.1.4", "ERROR: weird"]);
        assert!(!t.contains("ALSA") && !t.contains("libudev") && t.contains("weird"), "{t}");
    }

    /// The real shape of a Linux setup.sh log: cargo prints `Downloaded <crate>` lines for alsa-sys, libudev-sys and pkg-config on EVERY
    /// build (review B1), so a failure that has nothing to do with them must show its own error, not the ALSA hint.
    const DOWNLOADED: [&str; 7] = [
        "== (d) building the game",
        "  Downloaded libudev-sys v0.1.4",
        "  Downloaded pkg-config v0.3.31",
        "  Downloaded alsa-sys v0.3.1",
        "  Downloaded alsa v0.9.1",
        "   Compiling alsa-sys v0.3.1",
        "   Compiling rac-rs v0.4.1",
    ];

    fn mk_linux(extra: &[&str]) -> String {
        let mut p = Progress::new(false);
        for l in DOWNLOADED.iter().chain(extra.iter()) {
            p.feed_line(l);
        }
        p.error_text_for(true)
    }

    #[test]
    fn downloaded_crates_never_cause_a_library_hint() {
        let t = mk_linux(&["ERROR: cargo build failed (rustc was killed, out of memory)"]);
        assert!(!t.contains("ALSA") && !t.contains("libudev") && !t.contains("Settings > Advanced"), "{t}");
        assert!(t.contains("cargo build failed (rustc was killed, out of memory)"), "the real error text stays: {t}");
        // a Downloaded line next to a real linker error: still the plain text of that error
        let t = mk_linux(&["error: linking with `cc` failed: exit status: 1"]);
        assert!(!t.contains("ALSA") && t.contains("linking with `cc` failed"), "{t}");
        // the real missing-library failure after the same prefix still gets its hint
        let t = mk_linux(&["error: failed to run custom build command for `alsa-sys v0.3.1`", "  The system library `alsa` required by crate `alsa-sys` was not found."]);
        assert!(t.contains("ALSA") && t.contains("libasound2-dev") && t.contains("failed to run custom build command"), "{t}");
        let t = mk_linux(&["  = note: /usr/bin/ld: cannot find -ludev: No such file or directory"]);
        assert!(t.contains("libudev") && !t.contains("ALSA"), "{t}");
    }

    #[test]
    fn network_failures_beat_the_zig_hint() {
        let mk = |lines: &[&str]| {
            let mut p = Progress::new(false);
            p.feed_line("== (a) tools");
            for l in lines {
                p.feed_line(l);
            }
            p.error_text_for(true)
        };
        for l in [
            "curl: (6) Could not resolve host: ziglang.org",
            "curl: (7) Failed to connect to ziglang.org port 443 after 3 ms: Connection refused",
            "ERROR: could not download zig-linux-x86_64-0.14.1.tar.xz",
            "wget: unable to resolve host address 'ziglang.org'",
            "curl: (28) Connection timed out after 30001 milliseconds",
            "ziglang.org: Temporary failure in name resolution",
        ] {
            let t = mk(&["  downloading the C linker (zig 0.14.1) ...", l]);
            assert!(t.contains("internet connection") && !t.contains("Use my system"), "{l}: {t}");
        }
        // a real zig failure (no network words) still gets the zig hint; `ziglang` / `zigzag` alone do not
        let t = mk(&["zig: error: unable to find library"]);
        assert!(t.contains("Use my system"), "{t}");
        let mut p = Progress::new(false);
        p.feed_line("== (d) building");
        p.feed_line("error: could not compile `zigzag` (lib) due to 1 previous error");
        let t = p.error_text_for(true);
        assert!(!t.contains("Use my system") && t.contains("zigzag"), "{t}");
    }

    #[test]
    fn windows_failure_texts_get_no_linux_hint() {
        // the same logs that give a hint on Linux give the plain old texts on Windows (review S5)
        for lines in [
            vec!["error: failed to run custom build command for `alsa-sys v0.3.1`", "  The system library `alsa` required by crate `alsa-sys` was not found.", "  Could not run `pkg-config --libs alsa`"],
            vec!["error: linker `cc` not found"],
            vec!["zig: error: unable to find library"],
        ] {
            let mut p = Progress::new(false);
            p.feed_line("== (d) building");
            for l in &lines {
                p.feed_line(l);
            }
            let w = p.error_text_for(false);
            assert!(!w.contains("ALSA") && !w.contains("Settings > Advanced") && !w.contains("last error line"), "{w}");
            assert!(w.contains(&p.last_error) && w.ends_with("Click Try again. If it keeps failing, click Copy details and share them when you ask for help."), "{w}");
        }
        // the Linux-only network words do not change the Windows text
        let mut p = Progress::new(false);
        p.feed_line("== (d) building");
        p.feed_line("curl: (7) Failed to connect: Connection refused");
        assert!(!p.error_text_for(false).contains("internet connection"));
        assert!(p.error_text_for(true).contains("internet connection"));
    }

    /// Review SF-B1: on Linux every failure of setup.sh's step (a) (the pre-checks and the system-toolchain checks) used to read
    /// "check your internet connection" and hid the real ERROR line. These are the real texts of setup.sh.
    const SETUP_SH_STEP_A_ERRORS: [&str; 12] = [
        "this setup is for Linux (uname says Darwin)",
        "this setup is for 64-bit Intel/AMD PCs (x86_64); this machine is aarch64",
        "cannot create /mnt/sd/rac/toolchain/tmp (is this folder writable?)",
        "programs cannot run from this folder (/run/media/deck/SD/rac): the drive is mounted without permission to run programs, or it is a FAT/exFAT drive. Use a folder on your Linux drive (for the Steam Deck: the internal drive, or an SD card formatted as ext4).",
        "this folder's drive cannot make symbolic links (/run/media/deck/SD/rac; FAT/exFAT?). Use a folder on your Linux drive (for the Steam Deck: the internal drive, or an SD card formatted as ext4).",
        "libasound.so.2 (sound) is missing, and the game cannot be built without it. Install it with: sudo apt install libasound2t64 (libasound2 on older Debian/Ubuntu)",
        "libudev.so.1 (controllers) is missing, and the game cannot be built without it. Install it with: sudo pacman -S systemd-libs",
        "cargo not found (install the rust toolchain: https://rustup.rs)",
        "python3 not found (the generators need it)",
        "no C compiler found (cc/gcc): install gcc, pkg-config and the libasound2-dev / libudev-dev packages, or drop --system-toolchain",
        "Rust 1.80.0 is too old (1.90 or newer is needed, 1.99 is what the game is tested with). Install a current Rust with rustup: https://rustup.rs",
        "the downloaded Python does not run on this system",
    ];

    fn step_a_failure(update: bool, err: &str, linux: bool) -> String {
        let mut p = Progress::new(update);
        p.feed_line(if update { "== (u) update" } else { "== (a) tools" });
        p.feed_line("  using your own Rust, Python and C compiler (--system-toolchain)");
        p.feed_line(&format!("ERROR: {err}"));
        p.error_text_for(linux)
    }

    #[test]
    fn linux_step_a_errors_show_the_real_setup_sh_text_not_the_internet_hint() {
        for e in SETUP_SH_STEP_A_ERRORS {
            let t = step_a_failure(false, e, true);
            assert!(t.starts_with("The step 'Download tools' did not finish."), "{e}: {t}");
            assert!(t.contains(e), "the ERROR text is shown: {e}\n{t}");
            assert!(!t.contains("internet"), "{e}: {t}");
            assert!(t.contains("Try again") && t.contains("Copy details"), "{e}: {t}");
            // setup.sh's own text is the answer: no extra library hint on top (the cc line names the -dev packages in passing)
            assert!(!t.contains("The sound library ALSA") && !t.contains("The device library libudev") && !t.contains("Settings > Advanced") && !t.contains("The last error line was"), "{e}: {t}");
        }
        // an update run (step (u), index 0) that fails the same way shows the text too
        let t = step_a_failure(true, "python3 not found (the update tool needs it)", true);
        assert!(t.contains("python3 not found (the update tool needs it)") && !t.contains("internet") && t.contains("Play old version"), "{t}");
        let t = step_a_failure(true, "tools/update_apply.py is missing (the update tool); download the release again", true);
        assert!(t.contains("update_apply.py is missing") && !t.contains("internet"), "{t}");
    }

    #[test]
    fn linux_step_a_network_failures_still_say_to_check_the_connection() {
        for e in [
            "could not download https://static.rust-lang.org/dist/rust-1.99.0.tar.xz (check the internet connection, then run setup.sh again)",
            "could not download https://ziglang.org/download/0.14.1/zig-linux-x86_64-0.14.1.tar.xz (check the internet connection, then run setup.sh again)",
        ] {
            let t = step_a_failure(false, e, true);
            assert!(t.contains("Please check your internet connection, then click Try again."), "{t}");
        }
        // curl's own words, as printed before setup.sh's ERROR line
        let mut p = Progress::new(false);
        p.feed_line("== (a) tools");
        p.feed_line("curl: (6) Could not resolve host: static.rust-lang.org");
        p.feed_line("ERROR: could not download https://static.rust-lang.org/dist/x.tar.xz (check the internet connection, then run setup.sh again)");
        assert!(p.error_text_for(true).contains("internet connection"));
        // a stop in step (a) with no ERROR line at all keeps the old network text
        let mut p = Progress::new(false);
        p.feed_line("== (a) tools");
        p.feed_line("  downloading Rust 1.99.0 ...");
        assert!(p.error_text_for(true).contains("internet connection"));
        // network words win over an unrelated ERROR line text of the same step
        let mut p = Progress::new(false);
        p.feed_line("== (a) tools");
        p.feed_line("curl: (7) Failed to connect to static.rust-lang.org port 443: Connection refused");
        p.feed_line("ERROR: stopped");
        assert!(p.error_text_for(true).contains("internet connection"));
    }

    #[test]
    fn windows_step_0_failures_keep_the_internet_text() {
        // the Windows texts do not change: step 0 is only downloads there, an ERROR line or not
        for e in SETUP_SH_STEP_A_ERRORS.iter().take(3).chain(["could not download https://x/y.zip"].iter()) {
            let t = step_a_failure(false, e, false);
            assert!(t.contains("Please check your internet connection, then click Try again. Setup continues where it stopped."), "{e}: {t}");
            assert!(!t.contains(e), "{t}");
        }
        let mut p = Progress::new(false);
        p.feed_line("== (a) tools");
        assert!(p.error_text_for(false).contains("internet connection"));
    }

    #[test]
    fn elapsed_format() {
        assert_eq!(format_elapsed(Duration::from_secs(7 * 60 + 12)), "07:12");
    }
}
