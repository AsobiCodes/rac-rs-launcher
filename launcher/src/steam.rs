//! Linux: "Add to Steam" (the Steam Deck and desktop Steam). The launcher does not edit Steam's files itself: it asks the install's own
//! `setup.sh` (`--add-to-steam`, `--remove-from-steam`, `--steam-status`), which runs `tools/steam_shortcut.py` (the same entry, fields and
//! number as setup.ps1 -AddToSteam on Windows). The entry starts the GAME (`build/release/rac-rs`), not the launcher, so it also starts from
//! Gaming Mode with a controller. Steam must be closed (it rewrites its list when it closes); the tool says so and changes nothing otherwise.
//! Compiled on every system and tested on every system; the Windows launcher never calls it (`ui::state_steam::SteamUi::shown`).
use crate::platform;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Add,
    Remove,
    /// Steam found? Is our entry in it?
    Status,
}

impl Mode {
    /// The switch of setup.sh.
    pub fn flag(self) -> &'static str {
        match self {
            Mode::Add => "--add-to-steam",
            Mode::Remove => "--remove-from-steam",
            Mode::Status => "--steam-status",
        }
    }
}

/// The exit codes of the tool (the same as setup.ps1): 0 done, 1 error, 2 Steam (or a Steam user) not found, 3 Steam is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Done,
    Failed,
    NoSteam,
    Running,
}

pub fn kind_of(code: Option<i32>) -> Kind {
    match code {
        Some(0) => Kind::Done,
        Some(2) => Kind::NoSteam,
        Some(3) => Kind::Running,
        _ => Kind::Failed,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub kind: Kind,
    /// What to tell the player (the tool's own sentences, in plain words).
    pub text: String,
}

/// Steam and our entry in it, as `--steam-status` reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub found: bool,
    pub has: bool,
}

/// The arguments after `sh`.
pub fn args(root: &Path, mode: Mode) -> Vec<String> {
    vec![root.join("setup.sh").to_string_lossy().to_string(), mode.flag().to_string(), "--no-pause".to_string()]
}

/// `found=1 has=0` -> State.
pub fn parse_state(text: &str) -> Option<State> {
    let mut found = None;
    let mut has = None;
    for tok in text.split_whitespace() {
        match tok {
            "found=1" => found = Some(true),
            "found=0" => found = Some(false),
            "has=1" => has = Some(true),
            "has=0" => has = Some(false),
            _ => {}
        }
    }
    Some(State { found: found?, has: has? })
}

/// The tool's output as one text: the lines joined, the "ERROR: " prefix of a setup.sh failure dropped.
pub fn clean_text(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).map(|l| l.strip_prefix("ERROR: ").unwrap_or(l)).collect();
    lines.join(" ")
}

pub fn outcome_from(code: Option<i32>, raw: &str) -> Outcome {
    let kind = kind_of(code);
    let mut text = clean_text(raw);
    if text.is_empty() && kind != Kind::Done {
        text = "Steam could not be changed.".to_string();
    }
    Outcome { kind, text }
}

/// Only a setup.sh that knows the switches (game 0.4.2 and newer) may be asked: an older one takes `--add-to-steam` for the path of the disc image.
pub fn available(root: &Path) -> bool {
    root.join("tools").join("steam_shortcut.py").is_file() && std::fs::read_to_string(root.join("setup.sh")).map(|t| t.contains("--add-to-steam")).unwrap_or(false)
}

/// Runs `sh <root>/setup.sh <flag> --no-pause` in the install folder and waits (the tool reads a few small files: well under a second;
/// 30 s at most, then it is stopped). Returns the exit code (None = no code) and what it printed.
pub fn run_raw(root: &Path, mode: Mode) -> Result<(Option<i32>, String), String> {
    if !root.join("setup.sh").is_file() {
        return Err("setup.sh was not found in the game folder. Use Repair / rebuild (Settings, Advanced) first.".to_string());
    }
    if !available(root) {
        return Err("This version of the game cannot be added to Steam yet. Update the game first (it needs v0.4.2 or newer).".to_string());
    }
    let mut cmd = Command::new(platform::shell_exe());
    cmd.args(args(root, mode)).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    platform::clean_worker_env(&mut cmd);
    platform::hidden(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("setup.sh could not be started: {e}"))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() > Duration::from_secs(30) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("setup.sh could not be waited for: {e}")),
        }
    };
    let mut text = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut text);
    }
    if let Some(mut e) = child.stderr.take() {
        let mut err = String::new();
        let _ = e.read_to_string(&mut err);
        if !err.trim().is_empty() {
            text.push('\n');
            text.push_str(&err);
        }
    }
    match status {
        Some(s) => Ok((s.code(), text)),
        None => Err("Steam did not answer in 30 seconds.".to_string()),
    }
}

/// Add or remove: what to show the player.
pub fn run(root: &Path, mode: Mode) -> Outcome {
    match run_raw(root, mode) {
        Ok((code, raw)) => outcome_from(code, &raw),
        Err(e) => Outcome { kind: Kind::Failed, text: e },
    }
}

/// Is Steam there, and is the game in it? None when the tool could not say.
pub fn state(root: &Path) -> Option<State> {
    match run_raw(root, Mode::Status) {
        Ok((Some(0), raw)) => parse_state(&raw),
        _ => None,
    }
}

/// The text of the Settings button for the current state.
pub fn button_label(st: Option<State>) -> &'static str {
    if st.map(|s| s.has).unwrap_or(false) {
        "Remove from Steam"
    } else {
        "Add to Steam"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switches_are_the_ones_of_setup_sh() {
        assert_eq!(Mode::Add.flag(), "--add-to-steam");
        assert_eq!(Mode::Remove.flag(), "--remove-from-steam");
        assert_eq!(Mode::Status.flag(), "--steam-status");
        let root = Path::new("/home/deck/Games/rac");
        let a = args(root, Mode::Add);
        assert_eq!(a, vec![root.join("setup.sh").to_string_lossy().to_string(), "--add-to-steam".to_string(), "--no-pause".to_string()]);
    }

    #[test]
    fn exit_codes_are_those_of_setup_ps1() {
        assert_eq!(kind_of(Some(0)), Kind::Done);
        assert_eq!(kind_of(Some(1)), Kind::Failed);
        assert_eq!(kind_of(Some(2)), Kind::NoSteam);
        assert_eq!(kind_of(Some(3)), Kind::Running);
        assert_eq!(kind_of(None), Kind::Failed);
        assert_eq!(kind_of(Some(130)), Kind::Failed);
    }

    #[test]
    fn the_status_line_is_read_and_junk_is_refused() {
        assert_eq!(parse_state("found=1 has=0\n"), Some(State { found: true, has: false }));
        assert_eq!(parse_state("found=1 has=1"), Some(State { found: true, has: true }));
        assert_eq!(parse_state("found=0 has=0\n"), Some(State { found: false, has: false }));
        assert_eq!(parse_state(""), None);
        assert_eq!(parse_state("found=1"), None);
        assert_eq!(parse_state("ERROR: python3 not found"), None);
    }

    #[test]
    fn the_players_text_is_the_tools_sentences() {
        let o = outcome_from(Some(3), "Steam is running. Close Steam first.\nOn the Steam Deck: switch to Desktop Mode.\n");
        assert_eq!(o.kind, Kind::Running);
        assert_eq!(o.text, "Steam is running. Close Steam first. On the Steam Deck: switch to Desktop Mode.");
        let o = outcome_from(Some(1), "ERROR: python3 not found (the Steam shortcut tool needs it)\n");
        assert_eq!((o.kind, o.text.as_str()), (Kind::Failed, "python3 not found (the Steam shortcut tool needs it)"));
        let o = outcome_from(Some(1), "");
        assert_eq!(o.text, "Steam could not be changed.");
        assert_eq!(outcome_from(Some(0), "").text, "", "a silent success stays silent");
    }

    #[test]
    fn the_button_offers_the_other_way_round_when_the_game_is_in_steam() {
        assert_eq!(button_label(None), "Add to Steam");
        assert_eq!(button_label(Some(State { found: true, has: false })), "Add to Steam");
        assert_eq!(button_label(Some(State { found: true, has: true })), "Remove from Steam");
    }

    #[test]
    fn a_folder_without_setup_sh_is_told_plainly() {
        let d = std::env::temp_dir().join(format!("racl_steam_nosetup_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let o = run(&d, Mode::Add);
        assert_eq!(o.kind, Kind::Failed);
        assert!(o.text.contains("setup.sh was not found"), "{}", o.text);
        assert_eq!(state(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A setup.sh from before the switches existed would read `--add-to-steam` as the disc image: it is never started.
    #[test]
    fn an_older_game_is_not_asked() {
        let d = std::env::temp_dir().join(format!("racl_steam_old_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("setup.sh"), "#!/bin/sh
echo started > started.txt
").unwrap();
        assert!(!available(&d));
        let o = run(&d, Mode::Add);
        assert_eq!(o.kind, Kind::Failed);
        assert!(o.text.contains("needs v0.4.2 or newer"), "{}", o.text);
        assert!(!d.join("started.txt").exists(), "an old setup.sh must never be run with the new switch");
        assert_eq!(state(&d), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The real call through `sh` with a stand-in setup.sh (Unix only): the exit code and the words reach the caller, and the status reads back.
    #[cfg(unix)]
    #[test]
    fn a_stand_in_setup_sh_answers_through_the_real_call() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("racl_steam_fake_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("tools")).unwrap();
        std::fs::write(d.join("tools").join("steam_shortcut.py"), "").unwrap();
        let script = d.join("setup.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\ncase \"$1\" in\n  --add-to-steam) echo 'Added it.'; exit 0 ;;\n  --remove-from-steam) echo 'Steam is running. Close Steam first.'; exit 3 ;;\n  --steam-status) echo 'found=1 has=1'; exit 0 ;;\nesac\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(run(&d, Mode::Add), Outcome { kind: Kind::Done, text: "Added it.".to_string() });
        assert_eq!(run(&d, Mode::Remove), Outcome { kind: Kind::Running, text: "Steam is running. Close Steam first.".to_string() });
        assert_eq!(state(&d), Some(State { found: true, has: true }));
        let _ = std::fs::remove_dir_all(&d);
    }
}
