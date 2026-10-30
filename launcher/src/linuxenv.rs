//! Linux first-start helpers: which system libraries the game opens when it starts (checked before Play, with the install line of the
//! player's distribution), and a build-time estimate from the machine's size. The logic is plain functions that compile and are tested
//! on every system; only the probes (`can_load`, `machine`) look at the real PC, and they say "fine" off Linux.
//! The library list and the package names are the ones of `setup.sh` (Preflight/CheckLib/PkgHint): keep the two in step.

/// A system library the game needs, with the package that holds it on each family of distributions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lib {
    pub so: &'static str,
    pub apt: &'static str,
    pub dnf: &'static str,
    pub pacman: &'static str,
    pub what: &'static str,
}

/// Opened when the game starts, on every desktop.
pub const CORE_LIBS: &[Lib] = &[
    Lib { so: "libasound.so.2", apt: "libasound2t64", dnf: "alsa-lib", pacman: "alsa-lib", what: "sound" },
    Lib { so: "libudev.so.1", apt: "libudev1", dnf: "systemd-libs", pacman: "systemd-libs", what: "controllers" },
    Lib { so: "libvulkan.so.1", apt: "libvulkan1", dnf: "vulkan-loader", pacman: "vulkan-icd-loader", what: "graphics (the Vulkan loader; your graphics driver brings it)" },
    Lib { so: "libxkbcommon.so.0", apt: "libxkbcommon0", dnf: "libxkbcommon", pacman: "libxkbcommon", what: "the window (keyboard)" },
];
/// Needed on top when the window opens on X11 (checked only when libX11 is there; winit loads libxkbcommon-x11 for X11 only, so a pure
/// Wayland system without it must still get Play).
pub const X11_LIBS: &[Lib] = &[
    Lib { so: "libxkbcommon-x11.so.0", apt: "libxkbcommon-x11-0", dnf: "libxkbcommon-x11", pacman: "libxkbcommon-x11", what: "the window (keyboard)" },
    Lib { so: "libX11-xcb.so.1", apt: "libx11-xcb1", dnf: "libX11-xcb", pacman: "libx11", what: "the window (X11)" },
    Lib { so: "libXcursor.so.1", apt: "libxcursor1", dnf: "libXcursor", pacman: "libxcursor", what: "the window (X11, mouse pointer)" },
    Lib { so: "libXi.so.6", apt: "libxi6", dnf: "libXi", pacman: "libxi", what: "the window (X11, mouse and keyboard input)" },
];
/// Neither X11 nor Wayland libraries found: the game needs a desktop.
pub const NO_DESKTOP: Lib = Lib { so: "libX11.so.6", apt: "libx11-6 libx11-xcb1 libxcursor1 libxi6", dnf: "libX11 libX11-xcb libXcursor libXi", pacman: "libx11 libxcursor libxi", what: "the window (a desktop with X11 or Wayland)" };

/// The libraries missing on this PC. `can_load` says whether a library file name opens.
pub fn missing_libs(can_load: &dyn Fn(&str) -> bool) -> Vec<Lib> {
    let mut out: Vec<Lib> = CORE_LIBS.iter().filter(|l| !can_load(l.so)).copied().collect();
    if can_load("libX11.so.6") {
        out.extend(X11_LIBS.iter().filter(|l| !can_load(l.so)).copied());
    } else if !can_load("libwayland-client.so.0") {
        out.push(NO_DESKTOP);
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Apt,
    Dnf,
    Pacman,
    /// SteamOS / Steam Deck: Arch with a read-only system folder; the libraries ship with it, so no install line is suggested.
    SteamOs,
    Other,
}

fn os_value<'a>(os_release: &'a str, key: &str) -> &'a str {
    os_release.lines().find_map(|l| l.trim().strip_prefix(key).and_then(|v| v.strip_prefix('='))).map(|v| v.trim().trim_matches('"')).unwrap_or("")
}

/// The package family of an `/etc/os-release` (ID and ID_LIKE, as `PkgHint` of setup.sh reads them).
pub fn family(os_release: &str) -> Family {
    let words: Vec<String> = format!("{} {}", os_value(os_release, "ID"), os_value(os_release, "ID_LIKE")).split_whitespace().map(|w| w.to_ascii_lowercase()).collect();
    let has = |n: &str| words.iter().any(|w| w == n);
    if has("steamos") {
        Family::SteamOs
    } else if has("arch") || has("manjaro") {
        Family::Pacman
    } else if has("fedora") || has("rhel") || has("centos") {
        Family::Dnf
    } else if has("debian") || has("ubuntu") {
        Family::Apt
    } else {
        Family::Other
    }
}

/// Debian-family releases whose sound library package is `libasound2t64` (Ubuntu 24.04 and later, Debian 13 and later); older ones have `libasound2`.
fn asound_is_t64(os_release: &str) -> bool {
    let code = [os_value(os_release, "UBUNTU_CODENAME"), os_value(os_release, "VERSION_CODENAME")].into_iter().find(|c| !c.is_empty()).unwrap_or("");
    if code.is_empty() {
        return true;
    }
    !matches!(code, "buster" | "bullseye" | "bookworm" | "focal" | "jammy" | "lunar" | "mantic" | "stretch" | "bionic" | "xenial")
}

/// The one line the player pastes into a terminal.
pub fn install_line(fam: Family, libs: &[Lib], os_release: &str) -> String {
    let pick = |l: &Lib| -> String {
        match fam {
            Family::Apt if l.so == "libasound.so.2" && !asound_is_t64(os_release) => "libasound2".to_string(),
            Family::Apt => l.apt.to_string(),
            Family::Dnf => l.dnf.to_string(),
            Family::Pacman => l.pacman.to_string(),
            Family::Other | Family::SteamOs => String::new(),
        }
    };
    // SteamOS has no install line: its system folder is read-only, and the libraries ship with it (see `missing_message`).
    if fam == Family::Other || fam == Family::SteamOs {
        let join = |f: &dyn Fn(&Lib) -> &str| -> String { dedup(libs.iter().flat_map(|l| f(l).split_whitespace().map(str::to_string)).collect()) };
        return format!("install {} (Debian/Ubuntu), {} (Fedora) or {} (Arch)", join(&|l| l.apt), join(&|l| l.dnf), join(&|l| l.pacman));
    }
    let names = dedup(libs.iter().flat_map(|l| pick(l).split_whitespace().map(str::to_string).collect::<Vec<_>>()).collect());
    match fam {
        Family::Apt => format!("sudo apt install {names}"),
        Family::Dnf => format!("sudo dnf install {names}"),
        Family::Pacman => format!("sudo pacman -S {names}"),
        Family::Other | Family::SteamOs => unreachable!(),
    }
}

fn dedup(v: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for s in v {
        if !seen.contains(&s) {
            seen.push(s);
        }
    }
    seen.join(" ")
}

/// The text shown instead of starting the game when libraries are missing.
pub fn missing_message(libs: &[Lib], os_release: &str) -> String {
    let list = libs.iter().map(|l| format!("  {} ({})", l.so, l.what)).collect::<Vec<_>>().join("\n");
    let head = format!("The game cannot start yet: this PC is missing system libraries it needs.\n\n{list}\n\n");
    let fam = family(os_release);
    if fam == Family::SteamOs {
        // SteamOS ships these libraries and its system folder is read-only: no package command, a way to the right place instead
        return format!("{head}SteamOS normally has these libraries already. Start this launcher in Desktop Mode (Steam button > Power > Switch to Desktop), install the latest SteamOS update (Settings > System), then press Play again. If it still says the same, please report it at github.com/AsobiCodes/rac-rs.");
    }
    format!("{head}Open a terminal and paste this line (it asks for your password), then press Play again:\n\n  {}", install_line(fam, libs, os_release))
}

/// Can the system open this library file name? (`dlopen`, the way the game's window and graphics libraries are opened.)
#[cfg(unix)]
pub fn can_load(so: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(so) else { return false };
    // SAFETY: dlopen/dlclose with a NUL-terminated name; the handle is closed at once. These libraries have no unload-time surprises
    // (they are the ones every desktop program opens).
    unsafe {
        let h = libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_LOCAL);
        if h.is_null() {
            return false;
        }
        libc::dlclose(h);
    }
    true
}
#[cfg(not(unix))]
pub fn can_load(_so: &str) -> bool {
    true
}

/// Before Play: the message to show when libraries are missing (Linux only; `None` = go ahead). `RAC_LAUNCHER_NO_LIB_CHECK=1` skips the check.
pub fn play_blocker() -> Option<String> {
    if cfg!(not(target_os = "linux")) || std::env::var_os("RAC_LAUNCHER_NO_LIB_CHECK").is_some() {
        return None;
    }
    let missing = missing_libs(&can_load);
    if missing.is_empty() {
        return None;
    }
    Some(missing_message(&missing, &std::fs::read_to_string("/etc/os-release").unwrap_or_default()))
}

// ------------------------------------------------------------------ how long the build takes

/// How many compiler processes `setup.sh` starts (AutoCargoJobs): the CPU threads, but one per 2.3 GB of free memory.
pub fn build_jobs(threads: usize, mem_available_kb: Option<u64>) -> usize {
    let by_mem = (mem_available_kb.unwrap_or(8_000_000) / 2_300_000).max(1) as usize;
    threads.max(1).min(by_mem)
}

/// `MemAvailable` of /proc/meminfo in kB.
pub fn mem_available_kb(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|l| l.strip_prefix("MemAvailable:")).and_then(|v| v.split_whitespace().next()).and_then(|n| n.parse().ok())
}

/// "from .. to .. minutes" for the build of a game install: measured 15 minutes with 5 compiler processes (a 24-thread PC with a 12 GB
/// virtual machine), about 30 minutes on 4 cores (the game README). Rounded to 5 minutes.
pub fn build_minutes(jobs: usize) -> (u32, u32) {
    let est = 2.0 + 65.0 / jobs.max(1) as f64;
    let r5 = |m: f64| ((m / 5.0).round() as u32).max(1) * 5;
    let lo = r5(est);
    let hi = r5(est * 1.6).max(lo + 5);
    (lo, hi)
}

/// The sentence for the install page on Linux. `keeps_awake`: the launcher can hold a sleep inhibitor while the build runs
/// (`inhibit_linux::available()`), so the player only has to keep the PC plugged in; otherwise the player must keep it awake too.
pub fn install_note(threads: usize, mem_available_kb: Option<u64>, keeps_awake: bool) -> String {
    let jobs = build_jobs(threads, mem_available_kb);
    let (lo, hi) = build_minutes(jobs);
    let head = format!("This takes about {lo} to {hi} minutes on this PC ({threads} CPU threads). You can keep using your PC");
    if keeps_awake {
        format!("{head}, but keep it plugged in until it is done. The launcher keeps it from going to sleep while it builds.")
    } else {
        format!("{head}, but keep it plugged in and awake until it is done: sleep would stop the build.")
    }
}

/// `install_note` for this PC, worked out once (free memory falls while the build runs, so the sentence must not change on the way).
pub fn install_note_here() -> &'static str {
    static NOTE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NOTE.get_or_init(|| {
        let (threads, mem) = machine();
        install_note(threads, mem, crate::inhibit_linux::available())
    })
}

/// (CPU threads, free memory in kB) of this PC.
pub fn machine() -> (usize, Option<u64>) {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mem = std::fs::read_to_string("/proc/meminfo").ok().and_then(|t| mem_available_kb(&t));
    (threads, mem)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBUNTU_24: &str = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\nVERSION_CODENAME=noble\nUBUNTU_CODENAME=noble\n";
    const UBUNTU_22: &str = "NAME=\"Ubuntu\"\nVERSION_ID=\"22.04\"\nID=ubuntu\nID_LIKE=debian\nVERSION_CODENAME=jammy\nUBUNTU_CODENAME=jammy\n";
    const MINT_22: &str = "NAME=\"Linux Mint\"\nVERSION_ID=\"22\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\nVERSION_CODENAME=wilma\nUBUNTU_CODENAME=noble\n";
    const FEDORA: &str = "NAME=\"Fedora Linux\"\nID=fedora\nVERSION_ID=41\n";
    const ARCH: &str = "NAME=\"Arch Linux\"\nID=arch\n";
    const STEAMOS: &str = "NAME=\"SteamOS\"\nID=steamos\nID_LIKE=arch\nVERSION_ID=3.6\n";

    #[test]
    fn families_follow_id_and_id_like() {
        assert_eq!(family(UBUNTU_24), Family::Apt);
        assert_eq!(family(MINT_22), Family::Apt);
        assert_eq!(family(FEDORA), Family::Dnf);
        assert_eq!(family(ARCH), Family::Pacman);
        assert_eq!(family(STEAMOS), Family::SteamOs, "SteamOS says ID_LIKE=arch too: it must win");
        assert_eq!(family("ID=nixos\n"), Family::Other);
        assert_eq!(family(""), Family::Other);
    }

    #[test]
    fn nothing_missing_when_everything_opens() {
        assert!(missing_libs(&|_| true).is_empty());
    }

    #[test]
    fn a_missing_vulkan_loader_gives_the_exact_line_per_distro() {
        let only_vulkan_gone = |so: &str| so != "libvulkan.so.1";
        let m = missing_libs(&only_vulkan_gone);
        assert_eq!(m.len(), 1);
        assert_eq!(install_line(Family::Apt, &m, UBUNTU_24), "sudo apt install libvulkan1");
        assert_eq!(install_line(Family::Dnf, &m, FEDORA), "sudo dnf install vulkan-loader");
        assert_eq!(install_line(Family::Pacman, &m, ARCH), "sudo pacman -S vulkan-icd-loader");
        assert!(!install_line(Family::SteamOs, &m, STEAMOS).contains("steamos-readonly"), "no read-only switch-off advice on a Deck");
        assert_eq!(install_line(Family::Other, &m, ""), "install libvulkan1 (Debian/Ubuntu), vulkan-loader (Fedora) or vulkan-icd-loader (Arch)");
    }

    #[test]
    fn the_sound_package_name_follows_the_release() {
        let m = missing_libs(&|so| so != "libasound.so.2");
        assert_eq!(install_line(Family::Apt, &m, UBUNTU_24), "sudo apt install libasound2t64");
        assert_eq!(install_line(Family::Apt, &m, UBUNTU_22), "sudo apt install libasound2");
        assert_eq!(install_line(Family::Apt, &m, MINT_22), "sudo apt install libasound2t64", "Mint 22 is built on Ubuntu noble");
        assert_eq!(install_line(Family::Apt, &m, "ID=debian\nVERSION_CODENAME=bookworm\n"), "sudo apt install libasound2");
    }

    #[test]
    fn x11_libraries_are_only_asked_for_when_libx11_is_there() {
        // minimal install: libX11 but not the other three (what winit complains about)
        let m = missing_libs(&|so| !so.starts_with("libXcursor") && so != "libXi.so.6" && so != "libX11-xcb.so.1");
        assert_eq!(m.len(), 3);
        assert_eq!(install_line(Family::Apt, &m, UBUNTU_24), "sudo apt install libx11-xcb1 libxcursor1 libxi6");
        // no X11 but Wayland: fine; neither: one line for a desktop
        assert!(missing_libs(&|so| so != "libX11.so.6").is_empty());
        let none = missing_libs(&|so| so != "libX11.so.6" && so != "libwayland-client.so.0");
        assert_eq!(none, vec![NO_DESKTOP]);
        assert_eq!(install_line(Family::Pacman, &none, ARCH), "sudo pacman -S libx11 libxcursor libxi");
    }

    #[test]
    fn names_are_listed_once() {
        let m = missing_libs(&|so| !so.starts_with("libxkbcommon"));
        assert_eq!(m.len(), 2);
        assert_eq!(install_line(Family::Pacman, &m, ARCH), "sudo pacman -S libxkbcommon libxkbcommon-x11");
        let msg = missing_message(&m, UBUNTU_24);
        assert!(msg.contains("libxkbcommon-x11.so.0 (the window (keyboard))") && msg.contains("sudo apt install libxkbcommon0 libxkbcommon-x11-0"), "{msg}");
    }

    /// winit loads libxkbcommon-x11 only on X11: a Wayland-only PC without libX11 must not be blocked by it.
    #[test]
    fn libxkbcommon_x11_is_only_asked_for_on_x11() {
        let wayland_only = |so: &str| so != "libX11.so.6" && so != "libxkbcommon-x11.so.0";
        assert!(missing_libs(&wayland_only).is_empty(), "no libX11, libwayland-client present: nothing missing");
        let x11 = |so: &str| so != "libxkbcommon-x11.so.0";
        let m = missing_libs(&x11);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].so, "libxkbcommon-x11.so.0");
        assert!(CORE_LIBS.iter().all(|l| l.so != "libxkbcommon-x11.so.0"));
    }

    /// A Deck gets no `steamos-readonly` / pacman advice (read-only system folder, libraries ship with it): Desktop Mode + an update.
    #[test]
    fn steamos_gets_no_package_command() {
        let m = missing_libs(&|so| so != "libvulkan.so.1");
        let msg = missing_message(&m, STEAMOS);
        assert!(msg.contains("libvulkan.so.1") && msg.contains("Desktop Mode"), "{msg}");
        assert!(!msg.contains("pacman") && !msg.contains("steamos-readonly") && !msg.contains("sudo"), "{msg}");
        assert!(missing_message(&m, ARCH).contains("sudo pacman -S vulkan-icd-loader"), "other Arch systems keep the command");
    }

    #[test]
    fn jobs_and_minutes_match_the_measurements() {
        assert_eq!(mem_available_kb("MemTotal: 1 kB\nMemAvailable:   12345678 kB\n"), Some(12_345_678));
        assert_eq!(mem_available_kb("MemTotal: 1 kB\n"), None);
        assert_eq!(build_jobs(24, Some(11_900_000)), 5, "the 24-thread PC with 12 GB: five compiler processes");
        assert_eq!(build_jobs(8, Some(32_000_000)), 8);
        assert_eq!(build_jobs(4, Some(1_000_000)), 1);
        assert_eq!(build_minutes(5), (15, 25));
        assert_eq!(build_minutes(4), (20, 30), "about half an hour on 4 cores");
        assert_eq!(build_minutes(16), (5, 10));
        assert_eq!(build_minutes(1), (65, 105));
        let note = install_note(8, Some(10_000_000), false);
        assert!(note.starts_with("This takes about 20 to 30 minutes on this PC (8 CPU threads)") && note.contains("plugged in and awake"), "{note}");
        let held = install_note(8, Some(10_000_000), true);
        assert!(held.starts_with("This takes about 20 to 30 minutes on this PC (8 CPU threads)") && held.contains("keep it plugged in until it is done"), "{held}");
        assert!(!held.contains("and awake"), "with the inhibitor in place the player is not asked to keep it awake: {held}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_real_probe_finds_libc_and_not_nonsense() {
        assert!(can_load("libc.so.6"));
        assert!(!can_load("libdefinitely-not-here.so.9"));
        assert!(!can_load("bad\0name"));
    }
}
