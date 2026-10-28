//! The few operating-system calls (Windows-only ones behind cfg(windows), with a plain fallback elsewhere).
use std::path::{Path, PathBuf};
use std::process::Command;

/// Keeps the single-instance lock while it lives.
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    /// The locked file (Linux): the lock ends with the process or when this is dropped.
    #[cfg(not(windows))]
    _lock: Option<std::fs::File>,
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Named mutex `Local\RacRsLauncher`; None = another launcher of this user is already open.
#[cfg(windows)]
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
        if h.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            windows_sys::Win32::Foundation::CloseHandle(h);
            return None;
        }
        Some(InstanceGuard { handle: h })
    }
}

/// Linux: an exclusive `flock` on a file in `$XDG_RUNTIME_DIR` (else the temp folder); the lock goes away with the process.
/// A lock file that cannot be made never blocks the launcher.
#[cfg(not(windows))]
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    use std::os::unix::io::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let path = dir.join(format!("{safe}-{}.lock", unsafe { libc::getuid() }));
    let Ok(f) = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&path) else {
        return Some(InstanceGuard { _lock: None });
    };
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        Some(InstanceGuard { _lock: Some(f) })
    } else if lock_is_held(std::io::Error::last_os_error().raw_os_error()) {
        None
    } else {
        // any other flock failure (ENOLCK on an NFS /tmp, ...) is not "another launcher is open": a broken lock never blocks the launcher
        Some(InstanceGuard { _lock: None })
    }
}

/// flock's answer "somebody else holds it" (EWOULDBLOCK, the same number as EAGAIN on Linux).
#[cfg(not(windows))]
fn lock_is_held(errno: Option<i32>) -> bool {
    errno == Some(libc::EWOULDBLOCK) || errno == Some(libc::EAGAIN)
}

/// A hidden child: no console window.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

/// Linux: the child leads its own process group, so one signal to the group reaches everything it started (cargo, rustc, python, zig).
pub fn own_process_group(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// Linux: the setup worker does not inherit `LD_PRELOAD`. Steam's overlay sets it for every program started from a game-mode launcher; the
/// preloaded 32-bit/64-bit libraries make each child of setup.sh print "wrong ELF class" errors, which the launcher would show as the failure.
pub fn clean_worker_env(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        cmd.env_remove("LD_PRELOAD");
    }
    cmd
}

/// The shell that runs setup.sh (always `sh`, so a lost exec bit does not matter).
pub fn shell_exe() -> PathBuf {
    PathBuf::from("sh")
}

pub fn powershell_exe() -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(r) => Path::new(&r).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
        None => PathBuf::from("powershell.exe"),
    }
}

/// Kills the child and everything it started (exactly that PID's tree, as the old window did).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let tk = match std::env::var_os("SystemRoot") {
            Some(r) => Path::new(&r).join("System32").join("taskkill.exe"),
            None => PathBuf::from("taskkill.exe"),
        };
        let _ = hidden(Command::new(tk).args(["/PID", &pid.to_string(), "/T", "/F"])).status();
    }
    #[cfg(unix)]
    {
        // the worker leads its own group (`own_process_group`): the whole group goes
        signal_group(pid, libc::SIGKILL);
    }
}

/// Stops a setup worker and everything it started. Windows: `kill_tree`. Linux: SIGTERM to the worker's process group, up to 2 s for it to
/// end (setup.sh traps the signal and stops its children), then SIGKILL to the group; the worker is reaped. Blocks for about 3 s at most.
pub fn terminate(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        kill_tree(child.id());
    }
    #[cfg(unix)]
    {
        stop_group(child, std::time::Duration::from_secs(2));
    }
}

/// True while any process of the group `pgid` exists (a zombie that was not reaped yet counts).
#[cfg(unix)]
pub fn group_alive(pgid: u32) -> bool {
    if pgid <= 1 {
        return false;
    }
    let r = unsafe { libc::kill(-(pgid as i32), 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
fn signal_group(pgid: u32, sig: i32) {
    if pgid > 1 {
        unsafe {
            libc::kill(-(pgid as i32), sig);
        }
    }
}

/// `child` must have been started with `own_process_group` (its pid is the group id).
#[cfg(unix)]
pub fn stop_group(child: &mut std::process::Child, grace: std::time::Duration) {
    let pgid = child.id();
    signal_group(pgid, libc::SIGTERM);
    let end = std::time::Instant::now() + grace;
    let mut reaped = false;
    while std::time::Instant::now() < end {
        if !reaped {
            reaped = matches!(child.try_wait(), Ok(Some(_)));
        }
        if reaped && !group_alive(pgid) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // the leader was reaped and the group looked empty at the end of the grace time: nothing left to kill, and the number may be reused
    if reaped && !group_alive(pgid) {
        return;
    }
    signal_group(pgid, libc::SIGKILL);
    // reap the leader, but never wait for ever (a process in uninterruptible sleep, e.g. on a stuck network drive, ignores SIGKILL for a while)
    let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !reaped && std::time::Instant::now() < end {
        reaped = matches!(child.try_wait(), Ok(Some(_)));
        if !reaped {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    let end = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while group_alive(pgid) && std::time::Instant::now() < end {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Opens a folder or a file with the system's default program.
pub fn open_path(p: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(p).spawn();
    }
}

/// Opens a web address in the default browser (only when the player clicked a link; never called by itself).
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(url).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

/// Opens the folder of a file and selects it (plain open on other systems).
pub fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        let mut a = std::ffi::OsString::from("/select,");
        a.push(p.as_os_str());
        let _ = Command::new("explorer.exe").arg(a).spawn();
    }
    #[cfg(not(windows))]
    {
        open_path(p.parent().unwrap_or(p));
    }
}

#[derive(Clone, Copy)]
pub enum Known {
    Documents,
    Desktop,
}

/// Documents / Desktop of the user (follows OneDrive redirects through the shell), with an environment fallback.
pub fn known_folder(k: Known) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, SHGetKnownFolderPath};
        let id = match k {
            Known::Documents => &FOLDERID_Documents,
            Known::Desktop => &FOLDERID_Desktop,
        };
        unsafe {
            let mut out: *mut u16 = std::ptr::null_mut();
            let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut out);
            if hr >= 0 && !out.is_null() {
                let mut n = 0;
                while *out.add(n) != 0 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, n));
                CoTaskMemFree(out as *const _);
                return Some(PathBuf::from(s));
            }
        }
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(Path::new(&home).join(match k {
        Known::Documents => "Documents",
        Known::Desktop => "Desktop",
    }))
}

/// The per-user data base folder: `%APPDATA%` on Windows; on Linux `$XDG_DATA_HOME` or `~/.local/share` (XDG base directories).
pub fn data_base() -> Option<PathBuf> {
    pick_data_base(cfg!(windows), std::env::var_os("APPDATA"), std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn pick_data_base(windows: bool, appdata: Option<std::ffi::OsString>, xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let nonempty = |v: Option<std::ffi::OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    if windows {
        return nonempty(appdata);
    }
    nonempty(xdg).filter(|p| p.has_root()).or_else(|| nonempty(home).map(|h| h.join(".local").join("share")))
}

/// Free bytes for the current user on the drive that holds `dir` (an existing folder). None = unknown.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
        return (ok != 0).then_some(free);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::statvfs(c.as_ptr(), &mut s) } == 0;
        ok.then(|| (s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
    }
}

/// Tests only: true makes `recycle` delete its (scratch) path for good, so UI tests never fill the user's Recycle Bin.
pub static TEST_NO_RECYCLE_BIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// To the Recycle Bin, never a hard delete (except under `TEST_NO_RECYCLE_BIN`).
pub fn recycle(p: &Path) -> Result<(), String> {
    if TEST_NO_RECYCLE_BIN.load(std::sync::atomic::Ordering::SeqCst) {
        let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        return r.map_err(|e| e.to_string());
    }
    trash::delete(p).map_err(|e| e.to_string())
}

/// A running exe cannot be opened for writing on Windows: true = the game is running (also when another launcher started it).
pub fn exe_in_use(exe: &Path) -> bool {
    #[cfg(windows)]
    {
        if !exe.is_file() {
            return false;
        }
        match std::fs::OpenOptions::new().write(true).open(exe) {
            Ok(_) => false,
            Err(e) => matches!(e.raw_os_error(), Some(5) | Some(32)),
        }
    }
    #[cfg(unix)]
    {
        if !exe.is_file() {
            return false;
        }
        // Linux: look at what every process of this user runs (/proc/<pid>/exe). Opening the file for writing is no test: some kernels let
        // that succeed on a running program, a read-only file or mount always fails, and the probe itself can make a start of the game fail.
        if let Some(r) = exe_running_in(Path::new("/proc"), exe) {
            return r;
        }
        // no readable /proc: a running program cannot be opened for writing (ETXTBSY); nothing is written or cut by the probe
        match std::fs::OpenOptions::new().write(true).open(exe) {
            Ok(_) => false,
            Err(e) => e.raw_os_error() == Some(libc::ETXTBSY),
        }
    }
}

/// Is `exe` the program of any process listed in `proc_dir` (`/proc`)? None = the folder cannot be read. A replaced file (an update
/// while the game still runs the old one) is listed as `<path> (deleted)` and counts too. Processes of other users cannot be read: skipped.
#[cfg(unix)]
fn exe_running_in(proc_dir: &Path, exe: &Path) -> Option<bool> {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let want = exe.canonicalize().ok()?.into_os_string().into_vec();
    for e in std::fs::read_dir(proc_dir).ok()? {
        let Ok(e) = e else { continue };
        if !e.file_name().as_bytes().iter().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(link) = std::fs::read_link(e.path().join("exe")) else { continue };
        let l = link.as_os_str().as_bytes();
        let l = l.strip_suffix(b" (deleted)").unwrap_or(l);
        if l == want.as_slice() {
            return Some(true);
        }
    }
    Some(false)
}

/// A desktop shortcut `<dir>\<name>.lnk` to `target` (the launcher), working folder `workdir`, with the icon.
pub fn create_shortcut(dir: &Path, name: &str, target: &Path, workdir: &Path, icon: Option<&Path>) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let mut sl = mslnk::ShellLink::new(target).map_err(|e| e.to_string())?;
        sl.set_working_dir(Some(workdir.to_string_lossy().to_string()));
        sl.set_name(Some(name.to_string()));
        let ico = icon.map(|p| p.to_path_buf()).unwrap_or_else(|| target.to_path_buf());
        sl.set_icon_location(Some(ico.to_string_lossy().to_string()));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let lnk = dir.join(format!("{name}.lnk"));
        sl.create_lnk(&lnk).map_err(|e| e.to_string())?;
        Ok(lnk)
    }
    #[cfg(not(windows))]
    {
        let _ = (name, icon);
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let icon_ref = write_launcher_icon(dir);
        let entry = dir.join(DESKTOP_FILE);
        std::fs::write(&entry, desktop_entry_text(SHORTCUT_TITLE, target, workdir, icon_ref.as_deref())).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755));
        }
        Ok(entry)
    }
}

/// File name of the Linux menu entry (in `~/.local/share/applications`).
pub const DESKTOP_FILE: &str = "rac-rs-launcher.desktop";
#[cfg(not(windows))]
const SHORTCUT_TITLE: &str = "Ratchet & Clank (rac-rs)";
#[cfg(not(windows))]
const LAUNCHER_PNG: &[u8] = include_bytes!("../assets/rac-rs-256.png");

/// Where the shortcut goes: Windows = the desktop; Linux = the applications menu (`$XDG_DATA_HOME/applications`).
pub fn shortcut_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        known_folder(Known::Desktop)
    } else {
        data_base().map(|d| d.join("applications"))
    }
}

/// Puts the launcher's icon next to the menu entry: `<data>/icons/hicolor/256x256/apps/` when `dir` is the real `applications` folder,
/// else beside the entry (tests). Returns what `Icon=` should say.
#[cfg(not(windows))]
fn write_launcher_icon(dir: &Path) -> Option<String> {
    let target = if dir.file_name().map(|n| n == "applications").unwrap_or(false) {
        dir.parent()?.join("icons").join("hicolor").join("256x256").join("apps")
    } else {
        dir.to_path_buf()
    };
    std::fs::create_dir_all(&target).ok()?;
    let png = target.join("rac-rs-launcher.png");
    std::fs::write(&png, LAUNCHER_PNG).ok()?;
    Some(png.to_string_lossy().to_string())
}

/// The `Exec=` program: in double quotes (spaces), with the Desktop Entry escapes: `"`, backtick, `$` get a backslash and `\` is doubled
/// (each of those backslashes is doubled again by the file's own string escapes), `%` is doubled.
pub fn desktop_exec_quote(p: &str) -> String {
    let mut q = String::from("\"");
    for c in p.chars() {
        match c {
            '"' | '`' | '$' => {
                q.push_str("\\\\");
                q.push(c);
            }
            '\\' => q.push_str("\\\\\\\\"),
            '%' => q.push_str("%%"),
            '\n' => q.push_str("\\n"),
            '\r' => q.push_str("\\r"),
            '\t' => q.push_str("\\t"),
            _ => q.push(c),
        }
    }
    q.push('"');
    q
}

/// A plain string value of a Desktop Entry key (`Path=`, `Icon=`): `\` is doubled, a newline / tab / carriage return is written as its
/// escape, so a folder name can never end the line and start another key.
pub fn desktop_string(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            _ => o.push(c),
        }
    }
    o
}

/// The text of the menu entry: the launcher opens in the game's folder; `Terminal=false`.
pub fn desktop_entry_text(name: &str, target: &Path, workdir: &Path, icon: Option<&str>) -> String {
    let mut t = String::from("[Desktop Entry]\nType=Application\nVersion=1.0\n");
    t.push_str(&format!("Name={name}\nComment=Install, update and play Ratchet & Clank (rac-rs)\n"));
    t.push_str(&format!("Exec={}\n", desktop_exec_quote(&target.to_string_lossy())));
    t.push_str(&format!("Path={}\n", desktop_string(&workdir.to_string_lossy())));
    if let Some(i) = icon {
        t.push_str(&format!("Icon={}\n", desktop_string(i)));
    }
    t.push_str("Terminal=false\nCategories=Game;\nStartupWMClass=rac-rs-launcher\n");
    t
}

/// "Windows 10.0.26200" style text for the problem report.
pub fn os_text() -> String {
    #[cfg(windows)]
    {
        if let Ok(o) = hidden(Command::new("cmd").args(["/C", "ver"])).output() {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(t) = std::fs::read_to_string("/etc/os-release") {
            if let Some(n) = t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")) {
                return n.trim().trim_matches('"').to_string();
            }
        }
    }
    std::env::consts::OS.to_string()
}

#[cfg(test)]
mod xdg_tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn data_base_follows_the_platform() {
        let o = |s: &str| Some(OsString::from(s));
        assert_eq!(pick_data_base(true, o("C:/roaming"), o("/x"), o("/home/a")), Some(PathBuf::from("C:/roaming")));
        assert_eq!(pick_data_base(false, o("C:/x"), o("/data"), o("/home/a")), Some(PathBuf::from("/data")));
        assert_eq!(pick_data_base(false, None, None, o("/home/a")), Some(PathBuf::from("/home/a").join(".local").join("share")));
        assert_eq!(pick_data_base(false, None, o("relative"), o("/home/a")), Some(PathBuf::from("/home/a").join(".local").join("share")));
        assert_eq!(pick_data_base(false, None, None, None), None);
    }

    #[test]
    fn menu_entry_values_cannot_start_another_key() {
        // Path= and Icon= are strings: `\` is doubled and line breaks are escaped (review N2)
        assert_eq!(desktop_string("/home/p/My Games"), "/home/p/My Games");
        assert_eq!(desktop_string("/home/p/a\\b"), "/home/p/a\\\\b");
        assert_eq!(desktop_string("/x\nExec=evil\ty\r"), "/x\\nExec=evil\\ty\\r");
        let t = desktop_entry_text("Ratchet & Clank (rac-rs)", Path::new("/opt/ra ch/rac-rs-launcher"), Path::new("/home/p/g\nExec=evil\\s"), Some("/home/p/i\ncon.png"));
        assert_eq!(t.lines().filter(|l| l.starts_with("Exec=")).count(), 1, "{t}");
        assert!(t.contains("\nPath=/home/p/g\\nExec=evil\\\\s\n") && t.contains("\nIcon=/home/p/i\\ncon.png\n"), "{t}");
        assert!(t.contains("\nExec=\"/opt/ra ch/rac-rs-launcher\"\n"), "{t}");
        // the program path is escaped too
        assert_eq!(desktop_exec_quote("/a b/c\nd"), "\"/a b/c\\nd\"");
        assert_eq!(desktop_exec_quote("/a/$b`c\"d\\e%f"), "\"/a/\\\\$b\\\\`c\\\\\"d\\\\\\\\e%%f\"");
    }

    #[cfg(unix)]
    #[test]
    fn a_running_program_is_found_by_its_proc_entry() {
        let d = std::env::temp_dir().join(format!("racl_proc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let bin = d.join("game dir");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("rac-rs");
        std::fs::write(&exe, b"x").unwrap();
        let exe = exe.canonicalize().unwrap();
        let proc_dir = d.join("proc");
        std::fs::create_dir_all(proc_dir.join("100")).unwrap();
        std::fs::create_dir_all(proc_dir.join("self")).unwrap();
        std::fs::create_dir_all(proc_dir.join("101")).unwrap();
        // another program, a non-number entry that points at the exe, and a process without a readable exe link
        std::os::unix::fs::symlink("/usr/bin/other", proc_dir.join("100/exe")).unwrap();
        std::os::unix::fs::symlink(&exe, proc_dir.join("self/exe")).unwrap();
        assert_eq!(exe_running_in(&proc_dir, &exe), Some(false), "only numbered entries are processes");
        // the game runs: a link to the exe
        std::os::unix::fs::symlink(&exe, proc_dir.join("101/exe")).unwrap();
        assert_eq!(exe_running_in(&proc_dir, &exe), Some(true));
        // an update replaced the file while the game still runs the old one: ` (deleted)` is stripped
        std::fs::remove_file(proc_dir.join("101/exe")).unwrap();
        std::os::unix::fs::symlink(format!("{} (deleted)", exe.display()), proc_dir.join("101/exe")).unwrap();
        assert_eq!(exe_running_in(&proc_dir, &exe), Some(true));
        // a folder that cannot be read = unknown, so the caller falls back to the open-for-writing probe
        assert_eq!(exe_running_in(&d.join("no_proc"), &exe), None);
        // the real /proc: this very test program is running, a plain file is not
        #[cfg(target_os = "linux")]
        {
            let me = std::env::current_exe().unwrap();
            assert!(exe_in_use(&me), "the test program itself is a running exe");
            assert!(!exe_in_use(&exe), "a file nobody runs");
            assert!(!exe_in_use(&d.join("missing")));
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn the_setup_worker_does_not_get_ld_preload() {
        let run = |clean: bool| {
            let mut c = Command::new("sh");
            c.args(["-c", "printf '[%s]' \"${LD_PRELOAD-unset}\""]).env("LD_PRELOAD", "libgameoverlay.so");
            if clean {
                clean_worker_env(&mut c);
            }
            String::from_utf8_lossy(&c.output().unwrap().stdout).to_string()
        };
        assert_eq!(run(false), "[libgameoverlay.so]");
        assert_eq!(run(true), "[unset]");
    }

    #[cfg(unix)]
    #[test]
    fn only_a_held_lock_means_already_open() {
        assert!(lock_is_held(Some(libc::EWOULDBLOCK)) && lock_is_held(Some(libc::EAGAIN)));
        assert!(!lock_is_held(Some(libc::ENOLCK)) && !lock_is_held(Some(libc::EIO)) && !lock_is_held(None));
        // the real thing: a second launcher is refused while the first lives, and allowed after
        let name = format!("Local_RacTest{}", std::process::id());
        let first = single_instance(&name);
        assert!(first.is_some());
        assert!(single_instance(&name).is_none(), "second instance");
        drop(first);
        // a parallel test may still hold a forked copy of the lock's file for a moment (until its exec): wait for it
        let mut again = single_instance(&name);
        for _ in 0..150 {
            if again.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            again = single_instance(&name);
        }
        assert!(again.is_some(), "the lock ended with the first launcher");
    }
}

