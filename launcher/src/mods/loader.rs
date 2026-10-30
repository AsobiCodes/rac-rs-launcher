//! The OmniLoader library the game loads for script mods: `omniloader.dll` on Windows, `libomniloader.so` on Linux, next to the game exe.
//! The launcher only looks at the file (it is installed by setup next to the exe, from `bundled/`): present or missing, and the version
//! and ABI from the small `<library>.json` next to it when setup.sh wrote one (Linux; the Windows DLL has none, its version is unknown).
use std::path::{Path, PathBuf};

/// The library's file name on a system.
pub fn lib_name_for(windows: bool) -> &'static str {
    if windows {
        "omniloader.dll"
    } else {
        "libomniloader.so"
    }
}

/// Where the library is: in the folder of the game exe.
pub fn lib_path(exe: &Path) -> PathBuf {
    exe.parent().unwrap_or_else(|| Path::new(".")).join(lib_name_for(cfg!(windows)))
}

/// What the launcher knows about the installed library.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub present: bool,
    /// `"version"` of the sidecar file, e.g. `0.1.0`.
    pub version: Option<String>,
    /// `"abi"` of the sidecar file, e.g. `1.6`.
    pub abi: Option<String>,
}

/// `version` and `abi` from the text of a `<library>.json` (`{"version":"0.1.0","abi":"1.6",...}`); anything unreadable is None.
pub fn parse_sidecar(text: &str) -> (Option<String>, Option<String>) {
    let Ok(j) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else { return (None, None) };
    let get = |k: &str| j.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 32).map(str::to_string);
    (get("version"), get("abi"))
}

/// The installed library next to `exe`.
pub fn info(exe: &Path) -> Info {
    let lib = lib_path(exe);
    let present = lib.is_file();
    let mut side = lib.clone().into_os_string();
    side.push(".json");
    let (version, abi) = std::fs::read_to_string(PathBuf::from(side)).map(|t| parse_sidecar(&t)).unwrap_or((None, None));
    Info { present, version, abi }
}

/// The line the Mods page shows when script mods are installed but the library is not next to the game.
pub fn missing_text(windows: bool) -> String {
    let name = lib_name_for(windows);
    if windows {
        format!("The mod loader ({name}) is not next to the game, so script mods are off. Run Setup again to put it back.")
    } else {
        format!("The mod loader ({name}) is not next to the game, so script mods are off. Run setup.sh again to put it back.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_is_named_for_its_system() {
        assert_eq!(lib_name_for(true), "omniloader.dll");
        assert_eq!(lib_name_for(false), "libomniloader.so");
        let p = lib_path(Path::new("game/build/release/rac-rs"));
        assert_eq!(p.parent().unwrap(), Path::new("game/build/release"));
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), lib_name_for(cfg!(windows)));
    }

    #[test]
    fn the_sidecar_gives_version_and_abi_and_never_fails() {
        assert_eq!(parse_sidecar(r#"{"version":"0.1.0","abi":"1.6","sha256":"x"}"#), (Some("0.1.0".into()), Some("1.6".into())));
        assert_eq!(parse_sidecar("\u{feff}{\"version\":\"0.2.1\"}"), (Some("0.2.1".into()), None));
        assert_eq!(parse_sidecar(r#"{"version":7,"abi":""}"#), (None, None));
        assert_eq!(parse_sidecar("not json"), (None, None));
        assert_eq!(parse_sidecar(""), (None, None));
    }

    #[test]
    fn info_sees_the_file_and_its_sidecar() {
        let d = std::env::temp_dir().join(format!("rac_loader_info_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let exe = d.join("rac-rs");
        assert_eq!(info(&exe), Info { present: false, version: None, abi: None });
        let lib = lib_path(&exe);
        std::fs::write(&lib, b"x").unwrap();
        assert_eq!(info(&exe), Info { present: true, version: None, abi: None });
        let mut side = lib.clone().into_os_string();
        side.push(".json");
        std::fs::write(PathBuf::from(side), r#"{"version":"0.1.0","abi":"1.6"}"#).unwrap();
        assert_eq!(info(&exe), Info { present: true, version: Some("0.1.0".into()), abi: Some("1.6".into()) });
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_missing_text_names_the_file_and_the_script() {
        assert!(missing_text(true).contains("omniloader.dll") && missing_text(true).contains("Setup"));
        assert!(missing_text(false).contains("libomniloader.so") && missing_text(false).contains("setup.sh"));
    }
}
