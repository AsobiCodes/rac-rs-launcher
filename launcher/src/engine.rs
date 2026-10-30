//! The install / repair / update engine: the setup script as a hidden child, its output parsed by `setup_parse::Progress`.
//!
//! Windows:
//!   powershell.exe -NoProfile -ExecutionPolicy Bypass -File setup.ps1 -Console -Iso <iso> -SkipHash [-ApplyUpdate -NewVersion X [-FinishOnly]
//!   -SkipUnblock [-TestUpdateBase D]] -NoShortcut [-Regenerate]
//! Linux (same protocol, see the setup.sh contract):
//!   sh setup.sh --iso <iso> --skip-hash --no-pause [--apply-update --new-version X [--finish-only] --skip-unblock [--test-update-base D]]
//!   [--regenerate] (--system-toolchain | --pinned-toolchain)
//!
//! The launcher itself makes the desktop shortcut (to the launcher), so the Windows worker always gets -NoShortcut (setup.sh has none).
//! On Linux the worker leads its own process group, so Cancel reaches cargo, rustc, python and zig too.
use crate::platform;
use crate::setup_parse::Progress;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::Instant;

#[derive(Clone, Debug, Default)]
pub struct EngineOpts {
    pub root: PathBuf,
    pub iso: PathBuf,
    /// Some((version, finish_only)) = `-ApplyUpdate`.
    pub update: Option<(String, bool)>,
    /// Repair / rebuild: `-Regenerate` (makes the game code again).
    pub regenerate: bool,
    pub test_update_base: Option<String>,
    /// Linux only (ignored on Windows): `--system-toolchain`, use the system's Rust, Python and C compiler instead of the pinned ones.
    pub system_toolchain: bool,
    /// More switches for flow tests (`-CheckOnly`, `-NoBuild`); also from the environment variable RAC_LAUNCHER_SETUP_ARGS.
    pub extra: Vec<String>,
}

/// The arguments after the program (`powershell.exe` on Windows, `sh` elsewhere).
pub fn build_args(o: &EngineOpts) -> Vec<String> {
    build_args_for(cfg!(windows), o)
}

/// `build_args` for either system (tests check both on any host).
pub fn build_args_for(windows: bool, o: &EngineOpts) -> Vec<String> {
    if windows {
        build_args_windows(o)
    } else {
        build_args_unix(o)
    }
}

fn build_args_windows(o: &EngineOpts) -> Vec<String> {
    let mut a: Vec<String> = ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"].iter().map(|s| s.to_string()).collect();
    a.push(o.root.join("setup.ps1").to_string_lossy().to_string());
    a.extend(["-Console", "-Iso"].map(String::from));
    a.push(o.iso.to_string_lossy().to_string());
    a.push("-SkipHash".into());
    if let Some((ver, finish)) = &o.update {
        a.extend(["-ApplyUpdate", "-NewVersion"].map(String::from));
        a.push(ver.clone());
        if *finish {
            a.push("-FinishOnly".into());
        }
        a.push("-SkipUnblock".into());
        if let Some(b) = &o.test_update_base {
            a.push("-TestUpdateBase".into());
            a.push(b.trim_end_matches('\\').to_string());
        }
    }
    a.push("-NoShortcut".into());
    if o.regenerate {
        a.push("-Regenerate".into());
    }
    a.extend(o.extra.iter().cloned());
    a
}

fn build_args_unix(o: &EngineOpts) -> Vec<String> {
    let mut a: Vec<String> = vec![o.root.join("setup.sh").to_string_lossy().to_string()];
    a.push("--iso".into());
    a.push(o.iso.to_string_lossy().to_string());
    a.extend(["--skip-hash", "--no-pause"].map(String::from));
    if let Some((ver, finish)) = &o.update {
        a.extend(["--apply-update", "--new-version"].map(String::from));
        a.push(ver.clone());
        if *finish {
            a.push("--finish-only".into());
        }
        a.push("--skip-unblock".into());
        if let Some(b) = &o.test_update_base {
            a.push("--test-update-base".into());
            let t = b.trim_end_matches('/');
            a.push(if t.is_empty() { b.clone() } else { t.to_string() });
        }
    }
    if o.regenerate {
        a.push("--regenerate".into());
    }
    // setup.sh remembers the mode in toolchain/system.txt and keeps it when no switch is given: the unticked box must say so
    // explicitly (--pinned-toolchain clears the memory), or switching the box off after one system-toolchain run would do nothing.
    a.push(if o.system_toolchain { "--system-toolchain" } else { "--pinned-toolchain" }.into());
    a.extend(o.extra.iter().cloned());
    a
}

pub fn env_extra_args() -> Vec<String> {
    std::env::var("RAC_LAUNCHER_SETUP_ARGS").map(|s| s.split_whitespace().map(String::from).collect()).unwrap_or_default()
}

/// A running (or finished) worker.
pub struct Engine {
    child: Child,
    rx: Receiver<String>,
    pub progress: Progress,
    pub started: Instant,
    /// Set once the worker ended and its output is read: the exit code (-1 = killed / unknown).
    pub done: Option<i32>,
    pub cancelled: bool,
    /// Linux: the sleep inhibitor held while the worker runs (G7, `inhibit_linux.rs`); None elsewhere or when the system has no helper.
    /// Released the moment the job ends or is cancelled, and by `Drop` with the engine.
    awake: Option<crate::inhibit_linux::Inhibit>,
}

fn pump<R: std::io::Read + Send + 'static>(r: R, tx: std::sync::mpsc::Sender<String>) {
    std::thread::spawn(move || {
        let mut br = BufReader::new(r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match br.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let s = String::from_utf8_lossy(&buf).to_string();
                    for part in s.split(['\r', '\n']).filter(|p| !p.is_empty()) {
                        if tx.send(part.to_string()).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
}

impl Engine {
    pub fn start(opts: &EngineOpts) -> std::io::Result<Engine> {
        let script = script_path(&opts.root);
        if !script.is_file() {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{} was not found in {}", crate::library::SETUP_SCRIPT, opts.root.display())));
        }
        let mut cmd = Command::new(if cfg!(windows) { platform::powershell_exe() } else { platform::shell_exe() });
        cmd.args(build_args(opts)).current_dir(&opts.root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        platform::hidden(&mut cmd);
        platform::own_process_group(&mut cmd);
        platform::clean_worker_env(&mut cmd);
        let mut child = cmd.spawn()?;
        let (tx, rx) = channel();
        if let Some(o) = child.stdout.take() {
            pump(o, tx.clone());
        }
        if let Some(e) = child.stderr.take() {
            pump(e, tx);
        }
        Ok(Engine { child, rx, progress: Progress::new(opts.update.is_some()), started: Instant::now(), done: None, cancelled: false, awake: crate::inhibit_linux::Inhibit::hold() })
    }

    /// Reads what the worker printed since the last call and checks whether it ended. Returns true when something changed.
    pub fn poll(&mut self) -> bool {
        if self.done.is_some() {
            return false;
        }
        let mut changed = false;
        let mut disconnected = false;
        loop {
            match self.rx.try_recv() {
                Ok(l) => {
                    self.progress.feed_line(&l);
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        self.progress.update_progress(Instant::now());
        if let Ok(Some(st)) = self.child.try_wait() {
            // the pipes close with the process; wait a moment for the last lines
            if disconnected || self.started.elapsed().as_millis() > 0 {
                let t = Instant::now();
                while t.elapsed().as_millis() < 500 {
                    match self.rx.try_recv() {
                        Ok(l) => self.progress.feed_line(&l),
                        Err(TryRecvError::Empty) => std::thread::sleep(std::time::Duration::from_millis(20)),
                        Err(TryRecvError::Disconnected) => break,
                    }
                }
            }
            let code = st.code().unwrap_or(-1);
            self.progress.exit_code = Some(code);
            if code == 0 {
                self.progress.finished = true;
                self.progress.step = 4;
            }
            self.progress.update_progress(Instant::now());
            self.done = Some(code);
            self.awake = None;
            changed = true;
        }
        changed
    }

    /// Cancel: the child and its descendants only.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        if self.done.is_none() {
            platform::terminate(&mut self.child);
            // The worker is stopped (and reaped on Linux): mark it ended, so `Drop` does not signal the same group a second time. By then
            // the number may belong to an unrelated process group, and a stuck member would block the UI for the grace time twice.
            self.done = Some(self.child.try_wait().ok().flatten().and_then(|s| s.code()).unwrap_or(-1));
        }
        self.awake = None;
    }

    pub fn running(&self) -> bool {
        self.done.is_none()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.done.is_none() {
            platform::terminate(&mut self.child);
        }
    }
}

/// Where the setup worker's script lives (for messages).
pub fn script_path(root: &Path) -> PathBuf {
    root.join(crate::library::SETUP_SCRIPT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> EngineOpts {
        EngineOpts { root: PathBuf::from("C:\\g\\rac-rs"), iso: PathBuf::from("D:\\iso\\Ratchet & Clank.iso"), ..Default::default() }
    }

    fn unix_opts() -> EngineOpts {
        EngineOpts { root: PathBuf::from("/home/p/Games/rac-rs"), iso: PathBuf::from("/home/p/iso/Ratchet & Clank.iso"), ..Default::default() }
    }

    #[test]
    fn install_args_match_the_old_window() {
        let a = build_args_for(true, &opts());
        let script = PathBuf::from("C:\\g\\rac-rs").join("setup.ps1").to_string_lossy().to_string();
        assert_eq!(a, vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &script, "-Console", "-Iso", "D:\\iso\\Ratchet & Clank.iso", "-SkipHash", "-NoShortcut"]);
    }

    #[test]
    fn update_and_repair_args() {
        let mut o = opts();
        o.update = Some(("0.4.0".into(), true));
        o.test_update_base = Some("E:\\base\\".into());
        o.regenerate = true;
        // the Linux-only switch never reaches the Windows command line
        o.system_toolchain = true;
        let a = build_args_for(true, &o).join(" ");
        assert!(a.contains("-SkipHash -ApplyUpdate -NewVersion 0.4.0 -FinishOnly -SkipUnblock -TestUpdateBase E:\\base -NoShortcut -Regenerate"), "{a}");
        assert!(!a.contains("system-toolchain") && !a.contains("SystemToolchain"), "{a}");
    }

    #[test]
    fn linux_install_args_follow_the_setup_sh_contract() {
        let a = build_args_for(false, &unix_opts());
        assert_eq!(
            a.iter().map(|s| s.replace('\\', "/")).collect::<Vec<_>>(),
            vec!["/home/p/Games/rac-rs/setup.sh", "--iso", "/home/p/iso/Ratchet & Clank.iso", "--skip-hash", "--no-pause", "--pinned-toolchain"]
        );
    }

    #[test]
    fn linux_update_repair_and_toolchain_args() {
        let mut o = unix_opts();
        o.update = Some(("0.4.1".into(), true));
        o.test_update_base = Some("/tmp/base/".into());
        o.regenerate = true;
        o.system_toolchain = true;
        o.extra = vec!["--check-only".into()];
        let a = build_args_for(false, &o).join(" ");
        assert!(
            a.ends_with("--skip-hash --no-pause --apply-update --new-version 0.4.1 --finish-only --skip-unblock --test-update-base /tmp/base --regenerate --system-toolchain --check-only"),
            "{a}"
        );
        // a plain install passes no update switches; the box off says --pinned-toolchain (setup.sh would otherwise keep a remembered
        // system mode: review B3), the box on says --system-toolchain, never both
        let mut p = unix_opts();
        let off = build_args_for(false, &p).join(" ");
        assert!(off.ends_with("--no-pause --pinned-toolchain") && !off.contains("--system-toolchain"), "{off}");
        p.system_toolchain = true;
        let on = build_args_for(false, &p).join(" ");
        assert!(on.ends_with("--no-pause --system-toolchain") && !on.contains("--pinned-toolchain"), "{on}");
        // the same for an update and a repair
        let mut u = unix_opts();
        u.update = Some(("0.4.1".into(), false));
        u.regenerate = true;
        assert!(build_args_for(false, &u).join(" ").ends_with("--skip-unblock --regenerate --pinned-toolchain"));
        // a base folder named "/" stays "/"
        let mut r = unix_opts();
        r.update = Some(("0.4.1".into(), false));
        r.test_update_base = Some("/".into());
        assert!(build_args_for(false, &r).join(" ").ends_with("--test-update-base / --pinned-toolchain"));
    }

    #[test]
    fn missing_script_is_an_error_not_a_panic() {
        let mut o = opts();
        o.root = std::env::temp_dir().join("racl_no_such_root");
        let e = Engine::start(&o).err().unwrap();
        assert!(e.to_string().contains(crate::library::SETUP_SCRIPT), "{e}");
    }

    #[cfg(unix)]
    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_eng_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Runs a fake setup.sh through the real Engine (own process group, merged pipes, parser) until it ends.
    #[cfg(unix)]
    fn run_fake(n: &str, script: &str, tweak: impl Fn(&mut EngineOpts)) -> (Engine, PathBuf) {
        let root = scratch(n);
        std::fs::write(root.join("setup.sh"), script).unwrap();
        let mut o = EngineOpts { root: root.clone(), iso: PathBuf::from("/tmp/none.iso"), ..Default::default() };
        tweak(&mut o);
        let mut e = Engine::start(&o).expect("start setup.sh");
        let t = Instant::now();
        while e.running() {
            e.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
            assert!(t.elapsed().as_secs() < 60, "fake setup.sh did not end:\n{}", e.progress.log.join("\n"));
        }
        (e, root)
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_setup_sh_install_is_read_like_the_real_one() {
        let script = "#!/bin/sh\n\
            echo \"argv: $*\" > argv.txt\n\
            printf '\\n== (a) tools\\n'\n\
            echo '  downloading Rust 1.99.0 (x86_64-unknown-linux-gnu, about 120 MB) ...'\n\
            echo '  downloading Python 3.14.8 (about 30 MB) ...'\n\
            echo '  downloading the C linker (zig 0.14.1, about 90 MB) ...'\n\
            printf '\\n== (b) disc\\n'\n\
            printf '\\n== (c) game code\\n'\n\
            echo '  level 1 done: veldin'\n\
            echo '  level 2 done: novalis'\n\
            printf '\\n== (d) build\\n'\n\
            echo 'expect-compiles: 3'\n\
            echo '   Compiling x v1.0.0'\n\
            echo '   Compiling y v1.0.0' >&2\n\
            printf '\\n== (e) play.cfg\\n'\n\
            printf '\\n== done\\n'\n";
        let (e, root) = run_fake("ok", script, |o| o.system_toolchain = true);
        assert_eq!(e.done, Some(0), "{}", e.progress.log.join("\n"));
        assert!(e.progress.finished);
        assert_eq!(e.progress.step, 4);
        assert_eq!(e.progress.levels, 2, "{:?}", e.progress.log);
        assert_eq!((e.progress.expect, e.progress.compiled), (3, 2), "stderr and stdout lines both count");
        assert!((e.progress.tools_sub - 0.8).abs() < 1e-9, "the C linker line moves the tools bar");
        assert!(e.progress.last_error.is_empty());
        assert_eq!(e.progress.progress.floor() as u32, 100);
        // setup.sh was run from its folder with the Linux switches
        let argv = std::fs::read_to_string(root.join("argv.txt")).unwrap();
        assert!(argv.trim_end().ends_with("--iso /tmp/none.iso --skip-hash --no-pause --system-toolchain"), "{argv}");
        assert!(!argv.contains("--pinned-toolchain"), "{argv}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_fake_setup_sh_shows_its_error_line() {
        let script = "#!/bin/sh\nprintf '\\n== (a) tools\\n'\necho 'ERROR: the thing broke (code 7)' >&2\nexit 1\n";
        let (e, _) = run_fake("fail", script, |_| {});
        assert_eq!(e.done, Some(1));
        assert!(!e.progress.finished);
        assert_eq!(e.progress.last_error, "the thing broke (code 7)");
    }

    #[cfg(unix)]
    #[test]
    fn terminate_stops_the_whole_process_group() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 300 & sleep 300 & wait"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        platform::own_process_group(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let pgid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(platform::group_alive(pgid));
        platform::terminate(&mut child);
        assert!(!platform::group_alive(pgid), "children of the worker survived");
        // the worker was reaped by terminate: waiting again still works and reports its (signal) status
        assert!(child.try_wait().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn cancel_ends_a_running_fake_setup_sh_and_its_children() {
        let root = scratch("cancel");
        std::fs::write(root.join("setup.sh"), "#!/bin/sh\nprintf '\\n== (a) tools\\n'\nsleep 300 &\nsleep 300 &\necho $$ > pid.txt\nwait\n").unwrap();
        let o = EngineOpts { root: root.clone(), iso: PathBuf::from("/tmp/none.iso"), ..Default::default() };
        let mut e = Engine::start(&o).unwrap();
        let t = Instant::now();
        while !root.join("pid.txt").is_file() && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        let pgid: u32 = std::fs::read_to_string(root.join("pid.txt")).unwrap().trim().parse().unwrap();
        assert!(platform::group_alive(pgid));
        e.cancel();
        assert!(e.cancelled);
        assert!(!platform::group_alive(pgid), "children of the worker survived the cancel");
        // cancel ends the worker for good: Drop must not signal the (maybe reused) group a second time (review S1)
        assert!(e.done.is_some() && !e.running(), "cancel marks the worker as ended");
        assert!(!e.poll());
    }

    /// The real "Stop everything" path: a setup.sh that traps TERM (like the real one while it works) and a child that ignores TERM.
    /// SIGTERM is not enough, so the group gets the SIGKILL after the grace time, and Cancel still returns in a few seconds.
    #[cfg(unix)]
    #[test]
    fn cancel_escalates_to_sigkill_for_workers_that_ignore_term() {
        let root = scratch("cancel_kill");
        std::fs::write(
            root.join("setup.sh"),
            "#!/bin/sh\ntrap '' TERM\nprintf '\\n== (d) build\\n'\n(trap '' TERM; exec sleep 300) &\nsleep 300 &\necho $$ > pid.txt\nwhile true; do sleep 1; done\n",
        )
        .unwrap();
        let o = EngineOpts { root: root.clone(), iso: PathBuf::from("/tmp/none.iso"), ..Default::default() };
        let mut e = Engine::start(&o).unwrap();
        let t = Instant::now();
        while !root.join("pid.txt").is_file() && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
        let pgid: u32 = std::fs::read_to_string(root.join("pid.txt")).unwrap().trim().parse().unwrap();
        assert!(platform::group_alive(pgid));
        let t = Instant::now();
        e.cancel();
        let took = t.elapsed();
        assert!(!platform::group_alive(pgid), "the group survived the SIGKILL");
        assert!(took.as_millis() >= 1500, "SIGTERM alone should not have been enough: {took:?}");
        assert!(took.as_millis() < 5000, "Cancel blocked too long: {took:?}");
        assert!(e.done.is_some());
        // dropping the ended engine takes no time at all
        let t = Instant::now();
        drop(e);
        assert!(t.elapsed().as_millis() < 200);
    }
}
