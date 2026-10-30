//! Linux: the launcher keeps a stable copy of itself, so the applications-menu entry (and later a Steam shortcut) never points into a
//! Downloads folder the player may empty. `ensure_stable_copy` copies the running program to `<data>/rac-rs-launcher/rac-rs-launcher`
//! unless it already runs from there; the download itself is never touched. A copy that is newer than the running program (a launcher
//! that updated itself) is kept. Compiled everywhere and tested on every system; only Linux calls it (`state.rs: create_shortcut`).
use std::path::{Path, PathBuf};

/// Folder name under the user's data folder (`~/.local/share`).
pub const DIR_NAME: &str = "rac-rs-launcher";
/// File name of the copy.
pub const FILE_NAME: &str = "rac-rs-launcher";

/// `<data>/rac-rs-launcher`.
pub fn stable_dir(data_base: &Path) -> PathBuf {
    data_base.join(DIR_NAME)
}

/// Whether the menu entry should start the stable copy of the launcher `running`: only when this launcher keeps its library in the
/// user's data folder (`DataMode::AppData`). A launcher with `portable.txt` beside it, or inside a game folder, keeps its library next
/// to itself, and the copy in `<data>/rac-rs-launcher` would open another, empty one ("the game vanished").
pub fn wants_stable_copy(running: &Path, appdata: Option<&Path>) -> bool {
    running.parent().is_some_and(|d| crate::library::data_dir(d, appdata).1 == crate::library::DataMode::AppData)
}

fn same_place(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// The program the menu entry should start. `running` is the launcher that is running now. Returns the stable copy (made or kept), or
/// an error text; the caller falls back to `running` then.
pub fn ensure_stable_copy(running: &Path, dir: &Path) -> Result<PathBuf, String> {
    let dest = dir.join(FILE_NAME);
    if same_place(running, &dest) {
        return Ok(dest);
    }
    let run_time = std::fs::metadata(running).and_then(|m| m.modified()).map_err(|e| format!("cannot read {}: {e}", running.display()))?;
    if let Ok(t) = std::fs::metadata(&dest).and_then(|m| m.modified()) {
        if t >= run_time {
            return Ok(dest);
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let part = dir.join(format!("{FILE_NAME}.part"));
    let _ = std::fs::remove_file(&part);
    std::fs::copy(running, &part).map_err(|e| format!("cannot copy the launcher to {}: {e}", dir.display()))?;
    // the copy keeps the build's time stamp (a plain copy is stamped "now"): a launcher downloaded later but built before that moment
    // must still count as newer than this copy next time
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&part) {
        let _ = f.set_modified(run_time);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    // a rename over a running program is fine on Linux (the old file stays open until that process ends)
    std::fs::rename(&part, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("cannot put the launcher in {}: {e}", dir.display())
    })?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_self_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn set_age(p: &Path, secs_ago: u64) {
        let f = std::fs::OpenOptions::new().write(true).open(p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
    }

    /// The copy is for launchers whose library lives in the data folder; a portable or in-game-folder launcher keeps its own library.
    #[test]
    fn only_an_appdata_launcher_gets_the_stable_copy() {
        let t = temp("wants");
        let app = t.join("share");
        let dl = t.join("Downloads");
        std::fs::create_dir_all(&dl).unwrap();
        let run = dl.join("rac-rs-launcher-linux-x64");
        std::fs::write(&run, b"x").unwrap();
        assert!(wants_stable_copy(&run, Some(&app)), "a plain download: the library is in the data folder");
        assert!(!wants_stable_copy(&run, None), "no data folder: the launcher keeps its files beside itself");
        let usb = t.join("usb");
        std::fs::create_dir_all(&usb).unwrap();
        std::fs::write(usb.join(crate::library::PORTABLE_MARKER), "").unwrap();
        assert!(!wants_stable_copy(&usb.join("rac-rs-launcher"), Some(&app)), "portable.txt: the library is beside the launcher");
        let game = t.join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("VERSION"), "0.4.2").unwrap();
        std::fs::write(game.join(crate::library::SETUP_SCRIPT), "").unwrap();
        assert!(!wants_stable_copy(&game.join("rac-rs-launcher"), Some(&app)), "inside a game folder");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_download_is_copied_and_left_in_place() {
        let t = temp("copy");
        let dl = t.join("Downloads");
        std::fs::create_dir_all(&dl).unwrap();
        let run = dl.join("rac-rs-launcher-linux-x64");
        std::fs::write(&run, b"LAUNCHER v1").unwrap();
        let dir = stable_dir(&t.join("share"));
        let got = ensure_stable_copy(&run, &dir).unwrap();
        assert_eq!(got, dir.join("rac-rs-launcher"));
        assert_eq!(std::fs::read(&got).unwrap(), b"LAUNCHER v1");
        assert!(run.is_file(), "the download stays");
        assert!(!dir.join("rac-rs-launcher.part").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&got).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn running_from_the_stable_place_changes_nothing() {
        let t = temp("same");
        let dir = stable_dir(&t);
        std::fs::create_dir_all(&dir).unwrap();
        let me = dir.join("rac-rs-launcher");
        std::fs::write(&me, b"ME").unwrap();
        assert_eq!(ensure_stable_copy(&me, &dir).unwrap(), me);
        assert_eq!(std::fs::read(&me).unwrap(), b"ME");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_newer_download_replaces_the_copy_an_older_one_does_not() {
        let t = temp("age");
        let dir = stable_dir(&t.join("share"));
        std::fs::create_dir_all(&dir).unwrap();
        let stable = dir.join("rac-rs-launcher");
        std::fs::write(&stable, b"UPDATED BY ITSELF").unwrap();
        set_age(&stable, 3600);
        let old = t.join("old-download");
        std::fs::write(&old, b"OLD").unwrap();
        set_age(&old, 86_400);
        assert_eq!(ensure_stable_copy(&old, &dir).unwrap(), stable);
        assert_eq!(std::fs::read(&stable).unwrap(), b"UPDATED BY ITSELF", "an older program never replaces the copy");
        let newer = t.join("new-download");
        std::fs::write(&newer, b"NEW").unwrap();
        set_age(&newer, 60);
        ensure_stable_copy(&newer, &dir).unwrap();
        assert_eq!(std::fs::read(&stable).unwrap(), b"NEW");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn the_copy_keeps_the_build_time_so_a_later_download_of_a_newer_build_still_wins() {
        let t = temp("stamp");
        let dir = stable_dir(&t.join("share"));
        let v1 = t.join("v1");
        std::fs::write(&v1, b"V1").unwrap();
        set_age(&v1, 1000);
        let got = ensure_stable_copy(&v1, &dir).unwrap();
        let t1 = std::fs::metadata(&v1).unwrap().modified().unwrap();
        assert_eq!(std::fs::metadata(&got).unwrap().modified().unwrap(), t1, "the copy has the build's time, not the copy time");
        // v2 was built after v1 but before the moment v1 was copied
        let v2 = t.join("v2");
        std::fs::write(&v2, b"V2").unwrap();
        set_age(&v2, 500);
        ensure_stable_copy(&v2, &dir).unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), b"V2");
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        let t = temp("err");
        let e = ensure_stable_copy(&t.join("nope"), &t.join("share")).unwrap_err();
        assert!(e.contains("cannot read"), "{e}");
        let _ = std::fs::remove_dir_all(&t);
    }
}
