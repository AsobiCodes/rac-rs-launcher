//! The file dialog (disc image, game folders, mod zips). On Windows the system dialog; on Linux the `rfd` crate asks the desktop portal
//! first and then runs `zenity`. When neither exists `rfd` answers "nothing picked", which looks exactly like Cancel, so this module
//! tells the two apart: a quick "nothing" on Linux without zenity is tried with `kdialog` (KDE) and otherwise reported as `Unavailable`,
//! and the pages then say "type the path in the box" instead of doing nothing.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickResult {
    Picked(PathBuf),
    /// The player closed the dialog.
    Cancelled,
    /// No dialog program exists on this system (the text is for the player).
    Unavailable(String),
}

/// What to ask for.
#[derive(Clone, Debug, Default)]
pub struct Ask {
    pub title: String,
    /// A folder (else a file).
    pub folder: bool,
    /// (name, extensions without the dot; `*` = every file).
    pub filters: Vec<(String, Vec<String>)>,
    pub start: Option<PathBuf>,
}

impl Ask {
    pub fn file(title: &str, filters: &[(&str, &[&str])], start: Option<PathBuf>) -> Ask {
        Ask { title: title.into(), folder: false, filters: filters.iter().map(|(n, e)| (n.to_string(), e.iter().map(|s| s.to_string()).collect())).collect(), start }
    }
    pub fn folder(title: &str, start: Option<PathBuf>) -> Ask {
        Ask { title: title.into(), folder: true, filters: Vec::new(), start }
    }
}

pub const NO_DIALOG: &str = "No file dialog is installed on this system (zenity or kdialog). Type or paste the path into the box instead.";

/// A dialog that fails to open answers within this time; a person needs longer to see it and click.
const QUICK: Duration = Duration::from_millis(600);

/// True where `rfd` can come back empty because no dialog program exists (Linux and the other unix systems except macOS).
const NEEDS_FALLBACK: bool = cfg!(all(unix, not(target_os = "macos")));

/// Opens the dialog and waits for the answer (call it on a thread).
pub fn pick(ask: &Ask) -> PickResult {
    let t0 = Instant::now();
    let got = rfd_pick(ask);
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let have = |name: &str| which_in(name, &path_var).is_some();
    match judge(NEEDS_FALLBACK, got, t0.elapsed(), have("zenity"), have("kdialog")) {
        Next::Done(r) => r,
        Next::TryKdialog => kdialog_run(ask),
    }
}

fn rfd_pick(ask: &Ask) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(ask.title.as_str());
    if let Some(s) = &ask.start {
        d = d.set_directory(s);
    }
    for (name, ext) in &ask.filters {
        d = d.add_filter(name.as_str(), ext);
    }
    if ask.folder {
        d.pick_folder()
    } else {
        d.pick_file()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    Done(PickResult),
    TryKdialog,
}

/// What an empty answer of `rfd` means. `fallback`: this system can lack a dialog program (see `NEEDS_FALLBACK`).
pub fn judge(fallback: bool, got: Option<PathBuf>, elapsed: Duration, have_zenity: bool, have_kdialog: bool) -> Next {
    if let Some(p) = got {
        return Next::Done(PickResult::Picked(p));
    }
    if !fallback || elapsed >= QUICK || have_zenity {
        return Next::Done(PickResult::Cancelled);
    }
    if have_kdialog {
        Next::TryKdialog
    } else {
        Next::Done(PickResult::Unavailable(NO_DIALOG.to_string()))
    }
}

/// The program `name` in the folders of `path_var` (a file; the executable bit is not checked).
pub fn which_in(name: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var).map(|d| d.join(name)).find(|p| p.is_file())
}

/// `kdialog --title T --getopenfilename START FILTER` / `--getexistingdirectory START`.
pub fn kdialog_args(ask: &Ask) -> Vec<String> {
    let start = ask.start.as_deref().unwrap_or(Path::new(".")).to_string_lossy().to_string();
    let mut a = vec!["--title".to_string(), ask.title.clone()];
    if ask.folder {
        a.push("--getexistingdirectory".into());
        a.push(start);
    } else {
        a.push("--getopenfilename".into());
        a.push(start);
        let filter: Vec<String> = ask
            .filters
            .iter()
            .map(|(name, ext)| {
                let pat = if ext.iter().any(|e| e == "*") { "*".to_string() } else { ext.iter().map(|e| format!("*.{e}")).collect::<Vec<_>>().join(" ") };
                format!("{pat}|{name}")
            })
            .collect();
        if !filter.is_empty() {
            a.push(filter.join("\n"));
        }
    }
    a
}

fn kdialog_run(ask: &Ask) -> PickResult {
    match std::process::Command::new("kdialog").args(kdialog_args(ask)).output() {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() {
                PickResult::Cancelled
            } else {
                PickResult::Picked(PathBuf::from(s))
            }
        }
        Ok(_) => PickResult::Cancelled,
        Err(_) => PickResult::Unavailable(NO_DIALOG.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAST: Duration = Duration::from_millis(30);
    const SLOW: Duration = Duration::from_secs(4);

    #[test]
    fn a_picked_path_is_always_picked() {
        let p = PathBuf::from("/x/y.iso");
        for fb in [true, false] {
            assert_eq!(judge(fb, Some(p.clone()), FAST, false, false), Next::Done(PickResult::Picked(p.clone())));
        }
    }

    #[test]
    fn nothing_after_a_long_time_is_a_cancel() {
        assert_eq!(judge(true, None, SLOW, false, false), Next::Done(PickResult::Cancelled));
        assert_eq!(judge(true, None, SLOW, false, true), Next::Done(PickResult::Cancelled));
    }

    #[test]
    fn a_quick_nothing_without_zenity_is_kdialog_or_unavailable() {
        assert_eq!(judge(true, None, FAST, false, true), Next::TryKdialog);
        match judge(true, None, FAST, false, false) {
            Next::Done(PickResult::Unavailable(m)) => assert!(m.contains("Type or paste the path"), "{m}"),
            other => panic!("{other:?}"),
        }
        // zenity exists: a quick nothing is a cancel (or a zenity failure we cannot tell apart); nothing else is tried
        assert_eq!(judge(true, None, FAST, true, true), Next::Done(PickResult::Cancelled));
        // systems with native dialogs (Windows): never "unavailable"
        assert_eq!(judge(false, None, FAST, false, false), Next::Done(PickResult::Cancelled));
    }

    #[test]
    fn kdialog_gets_the_right_words() {
        let iso = Ask::file("Pick the disc", &[("Disc image (*.iso)", &["iso"]), ("All files", &["*"])], Some(PathBuf::from("/home/p/isos")));
        assert_eq!(kdialog_args(&iso), ["--title", "Pick the disc", "--getopenfilename", "/home/p/isos", "*.iso|Disc image (*.iso)\n*|All files"]);
        let zip = Ask::file("Pick a mod", &[("Zip file (*.zip)", &["zip"])], None);
        assert_eq!(kdialog_args(&zip), ["--title", "Pick a mod", "--getopenfilename", ".", "*.zip|Zip file (*.zip)"]);
        let dir = Ask::folder("Pick the folder", Some(PathBuf::from("/home/p")));
        assert_eq!(kdialog_args(&dir), ["--title", "Pick the folder", "--getexistingdirectory", "/home/p"]);
    }

    #[test]
    fn programs_are_found_on_the_path_only_when_they_exist() {
        let d = std::env::temp_dir().join(format!("racl_pick_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("zenity"), b"#!/bin/sh\n").unwrap();
        let other = std::env::temp_dir().join("racl_pick_empty_nonexistent");
        let path = std::env::join_paths([other, d.clone()]).unwrap();
        assert_eq!(which_in("zenity", &path), Some(d.join("zenity")));
        assert_eq!(which_in("kdialog", &path), None);
        assert_eq!(which_in("zenity", std::ffi::OsStr::new("")), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
