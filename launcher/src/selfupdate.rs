//! The launcher updates only itself, from its own repo (AsobiCodes/rac-rs-launcher, latest release); game updates are the setup script's job.
//! Asset `rac-rs-launcher-windows-x64.exe` (Linux: `rac-rs-launcher-linux-x64`), installed over the running exe (`RAC-RS Launcher.exe` / `rac-rs-launcher`); a `.exe` asset is refused off Windows.
//! Steps of `apply`: download + checksum to `<exe>.new`; rename the RUNNING exe to `<exe>.old` (Windows allows that), rename `.new` to the exe name;
//! the caller starts the new exe with `--restarted` and quits; the new start deletes `.old` (`remove_old`).
use crate::releases::{self, Release, ReleaseError, ReleaseSource, Transfer};
use crate::update::compare_version;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The release asset for THIS system and CPU. Linux names carry the architecture, so an x86_64 program is never swapped in on an ARM machine
/// (the launcher would be gone: "Exec format error"). An architecture without a release asset gets a name no release has: it is never offered.
#[cfg(windows)]
pub const ASSET_NAME: &str = "rac-rs-launcher-windows-x64.exe";
#[cfg(all(not(windows), target_arch = "x86_64"))]
pub const ASSET_NAME: &str = "rac-rs-launcher-linux-x64";
#[cfg(all(not(windows), target_arch = "aarch64"))]
pub const ASSET_NAME: &str = "rac-rs-launcher-linux-arm64";
#[cfg(all(not(windows), not(any(target_arch = "x86_64", target_arch = "aarch64"))))]
pub const ASSET_NAME: &str = "rac-rs-launcher-linux-unsupported-cpu";
/// The launcher's own version (Cargo.toml).
pub const OWN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherUpdate {
    pub version: String,
    pub notes: String,
    pub asset: releases::Asset,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SelfState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(LauncherUpdate),
    /// Offline, rate limit, no repo yet, no matching asset: nothing is shown to the user.
    Unavailable(String),
}

/// A newer launcher in `rel`, if it has the asset for this platform. Notes keep only the text (no markdown cleanup: the UI shows them as plain lines).
pub fn offer_from(rel: &Release, own: &str) -> Result<Option<LauncherUpdate>, ReleaseError> {
    if compare_version(&rel.version(), own) != Ordering::Greater {
        return Ok(None);
    }
    let asset = rel.asset(ASSET_NAME).ok_or_else(|| ReleaseError::Bad(format!("the release has no {ASSET_NAME}")))?.clone();
    Ok(Some(LauncherUpdate { version: rel.version(), notes: rel.notes(), asset }))
}

pub fn check(src: &ReleaseSource, own: &str) -> Result<Option<LauncherUpdate>, ReleaseError> {
    offer_from(&src.latest()?, own)
}

/// Runs `check` on a thread; the answer lands in `slot`.
pub fn check_async(src: ReleaseSource, own: String, slot: Arc<Mutex<SelfState>>) {
    if let Ok(mut g) = slot.lock() {
        *g = SelfState::Checking;
    }
    std::thread::spawn(move || {
        let st = match check(&src, &own) {
            Ok(Some(u)) => SelfState::Available(u),
            Ok(None) => SelfState::UpToDate,
            Err(e) => SelfState::Unavailable(e.text()),
        };
        if let Ok(mut g) = slot.lock() {
            *g = st;
        }
    });
}

pub fn old_path(exe: &Path) -> PathBuf {
    PathBuf::from(format!("{}.old", exe.display()))
}
pub fn new_path(exe: &Path) -> PathBuf {
    PathBuf::from(format!("{}.new", exe.display()))
}

/// Downloads the update and swaps it in place of `exe` (the running launcher). Returns `exe` (now the new version) for the restart.
/// On any failure the old exe is back in place and the `.new` file is gone.
pub fn apply(src: &ReleaseSource, upd: &LauncherUpdate, exe: &Path, t: &Transfer) -> Result<PathBuf, ReleaseError> {
    apply_for(cfg!(windows), ASSET_NAME, src, upd, exe, t)
}

/// ELF `e_machine` of this build's CPU (what the downloaded program must be for).
fn elf_machine(arch: &str) -> Option<u16> {
    match arch {
        "x86_64" => Some(0x3E),
        "aarch64" => Some(0xB7),
        _ => None,
    }
}

/// Linux: the first bytes of the download must be a 64-bit little-endian ELF program for `arch`. Nothing else may take the launcher's place.
fn check_elf_head(head: &[u8], arch: &str) -> Result<(), String> {
    let want = elf_machine(arch).ok_or_else(|| format!("no launcher update exists for the {arch} processor"))?;
    if head.len() < 20 || head[..4] != [0x7f, b'E', b'L', b'F'] {
        return Err("the downloaded file is not a Linux program".to_string());
    }
    if head[4] != 2 || head[5] != 1 {
        return Err("the downloaded file is not a 64-bit Linux program".to_string());
    }
    let machine = u16::from_le_bytes([head[18], head[19]]);
    if machine != want {
        return Err(format!("the downloaded program is for another processor (code {machine:#x}, this PC needs {want:#x})"));
    }
    Ok(())
}

/// `apply` with the system named. `expected_asset` is the asset name of this system and CPU. A Windows `.exe` asset is never swapped in
/// on another system, and on Linux only the asset made for this CPU is (the downloaded file's ELF header is checked as well).
fn apply_for(windows: bool, expected_asset: &str, src: &ReleaseSource, upd: &LauncherUpdate, exe: &Path, t: &Transfer) -> Result<PathBuf, ReleaseError> {
    if !windows && upd.asset.name.to_ascii_lowercase().ends_with(".exe") {
        return Err(ReleaseError::Bad(format!("{} is a Windows program and cannot replace this launcher", upd.asset.name)));
    }
    if !windows && upd.asset.name != expected_asset {
        return Err(ReleaseError::Bad(format!("{} is not the launcher for this PC ({expected_asset}) and cannot replace it", upd.asset.name)));
    }
    let new = new_path(exe);
    let old = old_path(exe);
    releases::download(src, &upd.asset, &new, t)?;
    if !windows {
        let mut head = [0u8; 20];
        let got = std::fs::File::open(&new).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head)).is_ok();
        if let Err(m) = check_elf_head(if got { &head } else { &[] }, std::env::consts::ARCH) {
            let _ = std::fs::remove_file(&new);
            return Err(ReleaseError::Bad(m));
        }
        // runnable BEFORE anything is swapped: a failure here keeps the old launcher
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)) {
                let _ = std::fs::remove_file(&new);
                return Err(releases::io_error(&e));
            }
        }
    }
    let _ = std::fs::remove_file(&old); // an older leftover
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(releases::io_error(&e));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(releases::io_error(&e));
    }
    Ok(exe.to_path_buf())
}

/// Tests: the bytes of a launcher download that passes the Linux checks (a 64-bit ELF header for this CPU), then `payload`.
#[cfg(test)]
pub fn test_launcher_bytes(payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0];
    b.extend(elf_machine(std::env::consts::ARCH).unwrap_or(0x3E).to_le_bytes());
    b.extend_from_slice(payload);
    b
}

/// Deletes `<exe>.old` (the launcher before the update). One attempt; true when it is gone.
pub fn remove_old(exe: &Path) -> bool {
    let old = old_path(exe);
    !old.exists() || std::fs::remove_file(&old).is_ok()
}

/// At start: remove `.old`, retrying for ~15 s on a thread (the previous launcher may still be closing).
pub fn cleanup_old(exe: PathBuf) {
    if remove_old(&exe) {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if remove_old(&exe) {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering as AO;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_self_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }

    #[test]
    fn newer_release_is_offered_older_or_equal_is_not() {
        let d = scratch("offer");
        releases::write_test_release(&d, "v0.2.0", "- better\n- faster", &[(ASSET_NAME, b"NEWEXE")], &good);
        let src = ReleaseSource::Folder(d.clone());
        let u = check(&src, "0.1.0").unwrap().unwrap();
        assert_eq!((u.version.as_str(), u.notes.as_str()), ("0.2.0", "- better\n- faster"));
        assert_eq!(check(&src, "0.2.0").unwrap(), None);
        assert_eq!(check(&src, "0.10.0").unwrap(), None);
        releases::write_test_release(&d, "v0.2.0", "", &[("other.txt", b"x")], &good);
        assert!(matches!(check(&src, "0.1.0"), Err(ReleaseError::Bad(_))));
        assert_eq!(check(&ReleaseSource::Folder(d.join("nope")), "0.1.0"), Err(ReleaseError::NotFound));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn update_swaps_the_exe_keeps_old_next_to_it_and_cleans_up() {
        let rel = scratch("rel");
        let app = scratch("app");
        let exe = app.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLDEXE").unwrap();
        std::fs::write(old_path(&exe), b"ANCIENT").unwrap();
        let bytes = test_launcher_bytes(b"NEWEXE-bytes");
        releases::write_test_release(&rel, "v0.2.0", "notes", &[(ASSET_NAME, bytes.as_slice())], &good);
        let src = ReleaseSource::Folder(rel.clone());
        let upd = check(&src, "0.1.0").unwrap().unwrap();
        let t = Transfer::default();
        assert_eq!(apply(&src, &upd, &exe, &t).unwrap(), exe);
        assert_eq!(std::fs::read(&exe).unwrap(), bytes);
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"OLDEXE");
        assert!(!new_path(&exe).exists());
        assert!(remove_old(&exe) && !old_path(&exe).exists());
        assert_eq!(t.done.load(AO::Relaxed), bytes.len() as u64);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755, "the new launcher is runnable");
        }
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn failed_update_leaves_the_old_exe_alone() {
        let rel = scratch("failrel");
        let app = scratch("failapp");
        let exe = app.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLDEXE").unwrap();
        releases::write_test_release(&rel, "v0.2.0", "", &[(ASSET_NAME, b"NEW")], &|_, _| Some(format!("sha256:{}", "a".repeat(64))));
        let src = ReleaseSource::Folder(rel.clone());
        let upd = check(&src, "0.1.0").unwrap().unwrap();
        assert_eq!(apply(&src, &upd, &exe, &Transfer::default()), Err(ReleaseError::DigestMismatch));
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLDEXE");
        assert!(!old_path(&exe).exists() && !new_path(&exe).exists());
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn a_windows_exe_never_replaces_a_non_windows_launcher() {
        let rel = scratch("exe_rel");
        let app = scratch("exe_app");
        let exe = app.join("rac-rs-launcher");
        std::fs::write(&exe, b"OLD").unwrap();
        releases::write_test_release(&rel, "v0.2.0", "", &[("rac-rs-launcher-windows-x64.exe", b"MZ")], &good);
        let src = ReleaseSource::Folder(rel.clone());
        let asset = src.latest().unwrap().asset("rac-rs-launcher-windows-x64.exe").unwrap().clone();
        let upd = LauncherUpdate { version: "0.2.0".into(), notes: String::new(), asset };
        let r = apply_for(false, "rac-rs-launcher-linux-x64", &src, &upd, &exe, &Transfer::default());
        assert!(matches!(r, Err(ReleaseError::Bad(ref m)) if m.contains("Windows")), "{r:?}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLD");
        assert!(!old_path(&exe).exists() && !new_path(&exe).exists());
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    /// Review S2: a Linux launcher only takes the asset made for its CPU, and only a real ELF program for it.
    #[test]
    fn a_linux_launcher_only_takes_a_program_made_for_this_cpu() {
        let rel = scratch("cpu_rel");
        let app = scratch("cpu_app");
        let exe = app.join("rac-rs-launcher");
        std::fs::write(&exe, b"OLD").unwrap();
        let elf_arm = {
            let mut b = test_launcher_bytes(b"payload");
            b[18] = 0xB7; // aarch64
            b[19] = 0;
            b
        };
        let elf_x64 = {
            let mut b = test_launcher_bytes(b"payload");
            b[18] = 0x3E;
            b[19] = 0;
            b
        };
        releases::write_test_release(
            &rel,
            "v0.2.0",
            "",
            &[("rac-rs-launcher-linux-arm64", elf_arm.as_slice()), ("rac-rs-launcher-linux-x64", elf_x64.as_slice()), ("rac-rs-launcher-linux-x64-text", b"#!/bin/sh\n")],
            &good,
        );
        let src = ReleaseSource::Folder(rel.clone());
        let r = src.latest().unwrap();
        let upd = |name: &str| LauncherUpdate { version: "0.2.0".into(), notes: String::new(), asset: r.asset(name).unwrap().clone() };
        let t = Transfer::default();
        let untouched = |exe: &Path| {
            assert_eq!(std::fs::read(exe).unwrap(), b"OLD");
            assert!(!old_path(exe).exists() && !new_path(exe).exists(), "no .old / .new left behind");
        };
        // an asset of another CPU is refused before anything is downloaded
        let e = apply_for(false, "rac-rs-launcher-linux-x64", &src, &upd("rac-rs-launcher-linux-arm64"), &exe, &t);
        assert!(matches!(e, Err(ReleaseError::Bad(ref m)) if m.contains("not the launcher for this PC")), "{e:?}");
        untouched(&exe);
        assert_eq!(t.done.load(AO::Relaxed), 0);
        // the right name but a script / wrong CPU inside is refused after the download, the old launcher stays
        let e = apply_for(false, "rac-rs-launcher-linux-x64-text", &src, &upd("rac-rs-launcher-linux-x64-text"), &exe, &t);
        assert!(matches!(e, Err(ReleaseError::Bad(ref m)) if m.contains("not a Linux program")), "{e:?}");
        untouched(&exe);
        let other = if std::env::consts::ARCH == "x86_64" { "rac-rs-launcher-linux-arm64" } else { "rac-rs-launcher-linux-x64" };
        if matches!(std::env::consts::ARCH, "x86_64" | "aarch64") {
            let e = apply_for(false, other, &src, &upd(other), &exe, &Transfer::default());
            assert!(matches!(e, Err(ReleaseError::Bad(ref m)) if m.contains("another processor")), "{e:?}");
            untouched(&exe);
        }
        // the program for this CPU goes in
        let mine = if std::env::consts::ARCH == "aarch64" { "rac-rs-launcher-linux-arm64" } else { "rac-rs-launcher-linux-x64" };
        if matches!(std::env::consts::ARCH, "x86_64" | "aarch64") {
            apply_for(false, mine, &src, &upd(mine), &exe, &Transfer::default()).unwrap();
            assert_eq!(std::fs::read(&exe).unwrap().len(), elf_x64.len());
            assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"OLD");
        }
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn the_elf_header_check() {
        let mut h = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0x3E, 0];
        assert_eq!(check_elf_head(&h, "x86_64"), Ok(()));
        assert!(check_elf_head(&h, "aarch64").unwrap_err().contains("another processor"));
        assert!(check_elf_head(&h, "riscv64").unwrap_err().contains("no launcher update"));
        h[18] = 0xB7;
        assert_eq!(check_elf_head(&h, "aarch64"), Ok(()));
        assert!(check_elf_head(b"MZ\x90\0", "x86_64").unwrap_err().contains("not a Linux program"));
        assert!(check_elf_head(&[], "x86_64").is_err());
        h[4] = 1; // 32-bit
        assert!(check_elf_head(&h, "aarch64").unwrap_err().contains("64-bit"));
        assert_eq!(ASSET_NAME.starts_with("rac-rs-launcher-"), true);
        if cfg!(not(windows)) {
            assert!(ASSET_NAME.contains(match std::env::consts::ARCH { "x86_64" => "x64", "aarch64" => "arm64", _ => "unsupported" }), "{ASSET_NAME}");
        }
    }

    #[test]
    fn async_check_reports_into_the_slot() {
        let d = scratch("async");
        releases::write_test_release(&d, "v3.0.0", "n", &[(ASSET_NAME, b"x")], &good);
        let slot = Arc::new(Mutex::new(SelfState::Idle));
        check_async(ReleaseSource::Folder(d.clone()), "0.1.0".into(), slot.clone());
        for _ in 0..100 {
            if !matches!(*slot.lock().unwrap(), SelfState::Checking) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(matches!(&*slot.lock().unwrap(), SelfState::Available(u) if u.version == "3.0.0"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
