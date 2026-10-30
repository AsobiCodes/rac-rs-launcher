//! Linux: keep the PC from going to sleep while the launcher builds or updates the game (gap G7). A build takes 15 to 60 minutes and a
//! suspend (a laptop lid, the Steam Deck's idle timer) would stop it half way. The launcher asks the system for a sleep inhibitor the
//! same way `systemd-inhibit` does for any program: it starts a small child (`systemd-inhibit ... sh -c <wait loop>`), keeps it as long as the
//! setup worker runs, and kills it when the job ends or is cancelled. No new crate and no D-Bus code of our own: a bus inhibitor lives only as
//! long as the connection that asked for it, which the command line helpers hold for us. Order of helpers: `systemd-inhibit` (every systemd
//! distribution, SteamOS included), then `gnome-session-inhibit` (GNOME without systemd). Neither present, or the helper is refused (for
//! example over ssh): nothing happens and the build runs as before. Windows and other systems: `Inhibit::hold()` is `None`.
//!
//! The held command is not `sleep infinity` but a loop that ends by itself a few seconds after the launcher process is gone, so a crash of
//! the launcher (release builds abort on panic) can never leave the PC unable to sleep until the next login.
//! `RAC_LAUNCHER_NO_INHIBIT=1` switches it off (tests, debugging). The pure parts (which helper, the command line, the PATH search) compile
//! and are tested on every system; only `hold()` looks at the real PC.
use std::ffi::OsStr;
use std::path::Path;

/// Shown by `systemd-inhibit --list` (the WHO and WHY columns).
pub const WHO: &str = "RAC-RS Launcher";
pub const WHY: &str = "building the game";

/// The command-line program that asks the system for the inhibitor and runs our wait loop under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Helper {
    Systemd,
    GnomeSession,
}

/// Tried in this order.
pub const HELPERS: &[Helper] = &[Helper::Systemd, Helper::GnomeSession];

impl Helper {
    pub fn program(self) -> &'static str {
        match self {
            Helper::Systemd => "systemd-inhibit",
            Helper::GnomeSession => "gnome-session-inhibit",
        }
    }
}

/// The first helper that `have` finds.
pub fn pick(have: &dyn Fn(&str) -> bool) -> Option<Helper> {
    HELPERS.iter().copied().find(|h| have(h.program()))
}

/// The shell text of the held command: sleeps in steps of `step_secs` while process `$1` (the launcher) exists.
pub fn wait_loop(step_secs: u32) -> String {
    format!("while kill -0 \"$1\" 2>/dev/null; do sleep {step_secs}; done")
}

/// The whole command line (program first) for `helper`: hold the inhibitor while process `launcher_pid` lives.
pub fn command_line(helper: Helper, launcher_pid: u32, step_secs: u32) -> Vec<String> {
    let hold = ["sh".to_string(), "-c".to_string(), wait_loop(step_secs), "sh".to_string(), launcher_pid.to_string()];
    let mut v: Vec<String> = match helper {
        // systemd-inhibit stops option parsing at the first word that is not an option, so `sh -c ...` is the command
        Helper::Systemd => ["systemd-inhibit", "--what=sleep", &format!("--who={WHO}"), &format!("--why={WHY}"), "--mode=block"].iter().map(|s| s.to_string()).collect(),
        Helper::GnomeSession => ["gnome-session-inhibit", "--app-id", "rac-rs-launcher", "--reason", WHY, "--inhibit", "suspend", "--"].iter().map(|s| s.to_string()).collect(),
    };
    v.extend(hold);
    v
}

fn is_runnable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Is there a runnable file `name` in one of the folders of `path_var` (empty entries are skipped: they would mean the current folder).
pub fn in_path(name: &str, path_var: &OsStr) -> bool {
    std::env::split_paths(path_var).filter(|d| !d.as_os_str().is_empty()).any(|d| is_runnable(&d.join(name)))
}

fn switched_off() -> bool {
    std::env::var_os("RAC_LAUNCHER_NO_INHIBIT").map(|v| !v.is_empty() && v != "0").unwrap_or(false)
}

/// The helper this PC has, or None (always None off Linux or with RAC_LAUNCHER_NO_INHIBIT set).
pub fn helper_here() -> Option<Helper> {
    if !cfg!(target_os = "linux") || switched_off() {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    pick(&|n| in_path(n, &path))
}

/// True when `Inhibit::hold()` has something to start here (the install page then says the launcher keeps the PC awake).
pub fn available() -> bool {
    helper_here().is_some()
}

/// A held inhibitor. Dropping it releases it (the helper and its wait loop are killed at once, a thread reaps them).
#[cfg(unix)]
pub struct Inhibit {
    child: Option<std::process::Child>,
}

#[cfg(not(unix))]
pub struct Inhibit;

#[cfg(unix)]
impl Inhibit {
    /// Starts the helper for this PC. None = no helper (or switched off) or it could not be started.
    pub fn hold() -> Option<Inhibit> {
        let helper = helper_here()?;
        Inhibit::spawn_line(&command_line(helper, std::process::id(), 5))
    }

    /// Runs `line` (program first) in its own process group with no input or output. Public for the tests.
    pub fn spawn_line(line: &[String]) -> Option<Inhibit> {
        use std::process::{Command, Stdio};
        let (prog, args) = line.split_first()?;
        let mut cmd = Command::new(prog);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        crate::platform::own_process_group(&mut cmd);
        crate::platform::clean_worker_env(&mut cmd);
        cmd.spawn().ok().map(|c| Inhibit { child: Some(c) })
    }

    /// The helper's process id (also its process group).
    pub fn pid(&self) -> u32 {
        self.child.as_ref().map(|c| c.id()).unwrap_or(0)
    }

    /// False once the helper ended (refused by the system, or killed).
    pub fn alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }
}

#[cfg(not(unix))]
impl Inhibit {
    pub fn hold() -> Option<Inhibit> {
        None
    }
}

#[cfg(unix)]
impl Drop for Inhibit {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            // SIGKILL to the helper's group: the system releases the inhibitor the moment the helper's process is gone. A helper that
            // already ended (refused by the system) is not signalled: its number may belong to somebody else by now.
            if matches!(c.try_wait(), Ok(None)) {
                crate::platform::kill_tree(c.id());
            }
            std::thread::spawn(move || {
                let _ = c.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_inhibit_is_tried_first_then_gnome_then_nothing() {
        assert_eq!(pick(&|_| true), Some(Helper::Systemd));
        assert_eq!(pick(&|n| n == "gnome-session-inhibit"), Some(Helper::GnomeSession));
        assert_eq!(pick(&|_| false), None);
    }

    #[test]
    fn the_systemd_line_shows_who_and_why_and_blocks_sleep_but_not_the_screen_lock() {
        let l = command_line(Helper::Systemd, 4242, 5);
        assert_eq!(&l[..5], ["systemd-inhibit", "--what=sleep", "--who=RAC-RS Launcher", "--why=building the game", "--mode=block"]);
        assert_eq!(&l[5..], ["sh", "-c", "while kill -0 \"$1\" 2>/dev/null; do sleep 5; done", "sh", "4242"]);
    }

    #[test]
    fn the_gnome_line_ends_options_before_the_command() {
        let l = command_line(Helper::GnomeSession, 7, 5);
        assert_eq!(l[0], "gnome-session-inhibit");
        let dd = l.iter().position(|a| a == "--").expect("the options end with --");
        assert_eq!(&l[dd + 1..dd + 3], ["sh", "-c"], "{l:?}");
        assert!(l.iter().any(|a| a == "suspend") && l.iter().any(|a| a == "building the game"));
        assert!(!l.iter().any(|a| a.contains("idle")), "only sleep/suspend keeps the build alive; the screen may still blank and lock: {l:?}");
        assert_eq!(l.last().unwrap(), "7");
    }

    #[test]
    fn the_path_search_finds_runnable_files_and_skips_empty_entries() {
        let d = std::env::temp_dir().join(format!("racl_inhibit_path_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("systemd-inhibit");
        std::fs::write(&f, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
            let plain = d.join("gnome-session-inhibit");
            std::fs::write(&plain, "x").unwrap();
            std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
            let p = std::env::join_paths([std::path::PathBuf::from(""), d.clone()]).unwrap();
            assert!(in_path("systemd-inhibit", &p));
            assert!(!in_path("gnome-session-inhibit", &p), "a file without the execute bit is not a helper");
        }
        #[cfg(not(unix))]
        {
            let p = std::env::join_paths([d.clone()]).unwrap();
            assert!(in_path("systemd-inhibit", &p));
        }
        assert!(!in_path("systemd-inhibit", OsStr::new("")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn nothing_is_held_off_linux() {
        if !cfg!(target_os = "linux") {
            assert!(!available());
            assert!(Inhibit::hold().is_none());
        }
    }

    #[cfg(unix)]
    fn wait_until(mut f: impl FnMut() -> bool, secs: u64) -> bool {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        f()
    }

    #[cfg(unix)]
    #[test]
    fn dropping_the_inhibitor_kills_the_helper_and_what_it_started() {
        // sh starts a sleep under it, like systemd-inhibit starts the wait loop
        let line: Vec<String> = ["sh", "-c", "sleep 60; sleep 60"].iter().map(|s| s.to_string()).collect();
        let mut h = Inhibit::spawn_line(&line).expect("sh starts");
        let pid = h.pid();
        assert!(pid > 1 && h.alive());
        assert!(crate::platform::group_alive(pid));
        drop(h);
        assert!(wait_until(|| !crate::platform::group_alive(pid), 5), "the helper's whole group must be gone");
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_that_cannot_start_gives_none() {
        assert!(Inhibit::spawn_line(&["/definitely/not/here/systemd-inhibit".to_string()]).is_none());
        assert!(Inhibit::spawn_line(&[]).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_wait_loop_ends_by_itself_when_the_launcher_is_gone() {
        use std::process::{Command, Stdio};
        // a stand-in launcher that lives 2 s; the loop (1 s steps) must end within ~3 s after it, with nobody killing it
        let mut fake_launcher = Command::new("sleep").arg("2").stdin(Stdio::null()).spawn().expect("sleep starts");
        let line: Vec<String> = vec!["sh".into(), "-c".into(), wait_loop(1), "sh".into(), fake_launcher.id().to_string()];
        let mut h = Inhibit::spawn_line(&line).expect("sh starts");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(h.alive(), "the launcher is still there: the loop keeps waiting");
        let _ = fake_launcher.wait();
        assert!(wait_until(|| !h.alive(), 4), "the launcher ended: the loop must end");
    }
}
