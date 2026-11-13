//! Reuse of the Rust/Python toolchain of another install (about 220 MB) when a game's setup starts.
//!
//! A game's `setup.ps1` / `setup.sh` may know the switch `-ToolchainFrom <folder>` / `--toolchain-from <dir>` (R&C 2 does; the public R&C 1 0.5.1 does
//! not, and PowerShell would stop on an unknown switch). So the launcher passes it only when the setup script text names the switch, only when this
//! install has no toolchain of its own yet, and only for a source install that really has a `toolchain` folder with something in it.

use std::path::{Path, PathBuf};

/// The setup script of `root` names the toolchain switch (so passing it is safe). A missing or unreadable script = no.
pub fn script_supports(windows: bool, root: &Path) -> bool {
    let name = if windows { "setup.ps1" } else { "setup.sh" };
    let needle = if windows { "ToolchainFrom" } else { "--toolchain-from" };
    std::fs::read(root.join(name)).map(|b| String::from_utf8_lossy(&b).contains(needle)).unwrap_or(false)
}

/// A `toolchain` folder with at least one entry.
pub fn has_toolchain(root: &Path) -> bool {
    std::fs::read_dir(root.join("toolchain")).map(|mut d| d.next().is_some()).unwrap_or(false)
}

/// The install to take the toolchain from: the first of `others` (other installs of the library, not `own`) that has one.
/// None when `own` already has its own toolchain, or the setup script does not know the switch, or nobody has one.
pub fn pick_source(windows: bool, own: &Path, others: &[PathBuf]) -> Option<PathBuf> {
    if has_toolchain(own) || !script_supports(windows, own) {
        return None;
    }
    others.iter().find(|o| !crate::library::same_folder(o, own) && has_toolchain(o)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_tc_{}_{}", n, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn with_toolchain(d: &Path) {
        std::fs::create_dir_all(d.join("toolchain").join("cargo")).unwrap();
    }

    #[test]
    fn only_a_script_that_names_the_switch_gets_it() {
        let d = scratch("sw");
        // public R&C1 0.5.1 style: no such switch
        std::fs::write(d.join("setup.ps1"), "param([switch]$Console, [string]$Iso)\n").unwrap();
        std::fs::write(d.join("setup.sh"), "while [ $# -gt 0 ]; do case $1 in --iso) ;; esac; done\n").unwrap();
        assert!(!script_supports(true, &d) && !script_supports(false, &d));
        std::fs::write(d.join("setup.ps1"), "param([switch]$Console, [string]$ToolchainFrom = '')\n").unwrap();
        std::fs::write(d.join("setup.sh"), "case $1 in --toolchain-from) TC=$2 ;; esac\n").unwrap();
        assert!(script_supports(true, &d) && script_supports(false, &d));
        // the Windows script does not make the Linux one capable and the other way round
        std::fs::remove_file(d.join("setup.sh")).unwrap();
        assert!(script_supports(true, &d) && !script_supports(false, &d));
    }

    #[test]
    fn the_source_is_another_install_with_a_toolchain() {
        let (own, a, b) = (scratch("own"), scratch("a"), scratch("b"));
        std::fs::write(own.join("setup.ps1"), "$ToolchainFrom").unwrap();
        std::fs::write(own.join("setup.sh"), "--toolchain-from").unwrap();
        std::fs::create_dir_all(a.join("toolchain")).unwrap(); // empty folder does not count
        with_toolchain(&b);
        for windows in [true, false] {
            assert_eq!(pick_source(windows, &own, &[a.clone(), b.clone()]), Some(b.clone()));
            // the own folder is never its own source, nobody has one = none
            assert_eq!(pick_source(windows, &own, &[own.clone(), a.clone()]), None);
        }
        // an install that has its own toolchain keeps it
        with_toolchain(&own);
        assert_eq!(pick_source(true, &own, &[b.clone()]), None);
    }

    #[test]
    fn an_old_script_gets_no_source_even_when_another_install_has_one() {
        let (own, b) = (scratch("old"), scratch("srcb"));
        std::fs::write(own.join("setup.ps1"), "param([switch]$Console)").unwrap();
        with_toolchain(&b);
        assert_eq!(pick_source(true, &own, &[b]), None);
    }

    // ---- the decision against the REAL scripts (R&C 2 candidate in scratch_rc2_rel, public R&C 1 v0.5.1 zip in scratch_race) ----
    const R2_PS1: &str = r"E:\ratchet\scratch_rc2_rel\release_build\setup.ps1";
    const R2_SH: &str = r"E:\ratchet\scratch_rc2_rel\linux_root\setup.sh";
    const R1_ZIP: &str = r"E:\ratchet\scratch_race\release_out\update_base_v0.5.1\rac-rs-0.5.1.zip";

    fn zip_entry(name: &str) -> Option<Vec<u8>> {
        use std::io::Read;
        let mut z = zip::ZipArchive::new(std::fs::File::open(R1_ZIP).ok()?).ok()?;
        let mut f = z.by_name(name).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    }

    #[test]
    fn real_scripts_r2_gets_the_switch_and_public_r1_0_5_1_does_not() {
        let (Ok(r2ps1), Ok(r2sh), Some(r1ps1), Some(r1sh)) = (std::fs::read(R2_PS1), std::fs::read(R2_SH), zip_entry("rac-rs-0.5.1/setup.ps1"), zip_entry("rac-rs-0.5.1/setup.sh")) else {
            eprintln!("real setup scripts not on this PC: test skipped");
            return;
        };
        let (r2, r1, donor) = (scratch("real_r2"), scratch("real_r1"), scratch("real_donor"));
        std::fs::write(r2.join("setup.ps1"), &r2ps1).unwrap();
        std::fs::write(r2.join("setup.sh"), &r2sh).unwrap();
        std::fs::write(r1.join("setup.ps1"), &r1ps1).unwrap();
        std::fs::write(r1.join("setup.sh"), &r1sh).unwrap();
        with_toolchain(&donor);
        for windows in [true, false] {
            let from_r2 = pick_source(windows, &r2, &[donor.clone()]);
            let from_r1 = pick_source(windows, &r1, &[donor.clone()]);
            assert_eq!(from_r2, Some(donor.clone()), "windows={windows}");
            assert_eq!(from_r1, None, "windows={windows}");
            let mk = |root: &Path, src: Option<PathBuf>| crate::engine::EngineOpts { root: root.to_path_buf(), iso: PathBuf::from("d.iso"), toolchain_from: src, ..Default::default() };
            let a2 = crate::engine::build_args_for(windows, &mk(&r2, from_r2));
            let a1 = crate::engine::build_args_for(windows, &mk(&r1, from_r1));
            let sw = if windows { "-ToolchainFrom" } else { "--toolchain-from" };
            assert!(a2.iter().any(|x| x == sw), "{a2:?}");
            assert!(!a1.iter().any(|x| x == sw), "{a1:?}");
        }
    }

    /// Every switch the launcher passes (engine.rs, every option on) is named by the real R&C 2 scripts.
    #[test]
    fn real_r2_scripts_know_every_switch_the_launcher_passes() {
        let (Ok(ps1), Ok(sh)) = (std::fs::read_to_string(R2_PS1), std::fs::read_to_string(R2_SH)) else {
            eprintln!("real setup scripts not on this PC: test skipped");
            return;
        };
        let o = crate::engine::EngineOpts {
            root: PathBuf::from("R"),
            iso: PathBuf::from("d.iso"),
            update: Some(("0.1.1".into(), true)),
            regenerate: true,
            test_update_base: Some("B".into()),
            toolchain_from: Some(PathBuf::from("T")),
            ..Default::default()
        };
        for system in [false, true] {
            let o = crate::engine::EngineOpts { system_toolchain: system, ..o.clone() };
            for a in crate::engine::build_args_for(true, &o).iter().filter(|a| a.starts_with('-') && a.len() > 1 && a.as_bytes()[1].is_ascii_alphabetic()) {
                if a == "-NoProfile" || a == "-ExecutionPolicy" || a == "-File" {
                    continue;
                }
                assert!(ps1.contains(&format!("[switch]${}", &a[1..])) || ps1.contains(&format!("[string]${}", &a[1..])) || ps1.contains(&format!("[int]${}", &a[1..])), "setup.ps1 lacks {a}");
            }
            for a in crate::engine::build_args_for(false, &o).iter().filter(|a| a.starts_with("--")) {
                assert!(sh.contains(&format!("{a})")) || sh.contains(&format!("|{a})")) || sh.contains(&format!("{a}|")), "setup.sh lacks {a}");
            }
        }
    }
}
