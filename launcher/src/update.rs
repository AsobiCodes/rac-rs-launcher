//! Update check: the same sources and rules as setup.ps1 (Read-Remote, Get-ChangelogFor, Compare-Version).
//! VERSION and CHANGELOG.md come from the game's own repo (R&C 1: AsobiCodes/rac-rs, R&C 2: AsobiCodes/rac-rs2, `GameDef.repo`; branch main); the update itself is done by the setup.ps1 -ApplyUpdate worker
//! (see engine.rs). A test base (folder, file:// or http URL that serves VERSION and CHANGELOG.md) replaces GitHub.
use crate::games::{is_version, GameDef};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where a game's VERSION / CHANGELOG.md live: the raw files of its own repo (never another game's).
pub fn raw_base(repo: (&str, &str)) -> String {
    format!("https://raw.githubusercontent.com/{}/{}/main", repo.0, repo.1)
}

/// Why a remote file could not be read: it is not there (404 / missing file: the repo or release does not exist yet) or the read failed (offline, timeout...).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fetch {
    Missing,
    Failed,
}

/// -1 / 0 / 1 like [version]::CompareTo; unreadable parts count as 0. A test suffix ("0.4.0-rc1") sorts just below the same number without it.
pub fn compare_version(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> (Vec<u64>, bool) {
        let s = s.trim().trim_start_matches('v');
        let (num, pre) = match s.split_once('-') {
            Some((n, _)) => (n, true),
            None => (s, false),
        };
        (num.split('.').map(|x| x.parse().unwrap_or(0)).collect(), pre)
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    na.cmp(&nb).then_with(|| pb.cmp(&pa))
}

/// A test base that is a folder on this PC (a plain path or a `file://` URL); None for an http(s) URL.
/// `file:///E:/x` is a Windows drive path (the slash before the drive letter goes); `file:///tmp/x` keeps its root slash.
pub fn local_base(base: &str) -> Option<PathBuf> {
    let base = base.trim();
    if base.starts_with("http://") || base.starts_with("https://") {
        return None;
    }
    let Some(rest) = base.strip_prefix("file://") else { return Some(PathBuf::from(base)) };
    let b = rest.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        Some(PathBuf::from(&rest[1..]))
    } else {
        Some(PathBuf::from(rest))
    }
}

/// Text of VERSION / CHANGELOG.md of `repo` (or of the test `base`).
pub fn fetch_remote(base: Option<&str>, repo: (&str, &str), name: &str, timeout: Duration) -> Result<String, Fetch> {
    let default = raw_base(repo);
    let base = base.unwrap_or(&default);
    if let Some(dir) = local_base(base) {
        return std::fs::read_to_string(dir.join(name)).map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { Fetch::Missing } else { Fetch::Failed });
    }
    http_get(&format!("{}/{}", base.trim_end_matches('/'), name), timeout)
}

/// R&C 1's feed; None on any failure, never panics.
pub fn read_remote(base: Option<&str>, name: &str, timeout: Duration) -> Option<String> {
    fetch_remote(base, crate::releases::GAME_REPO, name, timeout).ok()
}

fn http_get(url: &str, timeout: Duration) -> Result<String, Fetch> {
    let cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).user_agent("rac-rs-launcher").http_status_as_error(false);
    #[cfg(windows)]
    let cfg = cfg.tls_config(ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build());
    let agent: ureq::Agent = cfg.build().into();
    let mut resp = agent.get(url).call().map_err(|_| Fetch::Failed)?;
    match resp.status().as_u16() {
        200..=299 => resp.body_mut().read_to_string().map_err(|_| Fetch::Failed),
        404 => Err(Fetch::Missing),
        _ => Err(Fetch::Failed),
    }
}

/// CHANGELOG.md has one section per version, newest first, starting with "## v<version>". Returns the versions newer than `from` up to `to`,
/// as the old window showed them ("Version x.y.z", bullets with a dot).
pub fn changelog_for(text: &str, from: &str, to: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut take = false;
    for l in text.lines() {
        if let Some(h) = l.strip_prefix("##") {
            let h = h.trim_start();
            let h = h.strip_prefix('v').unwrap_or(h);
            let ver: String = h.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            if is_version(&ver) {
                use std::cmp::Ordering::*;
                take = compare_version(&ver, from) == Greater && compare_version(&ver, to) != Greater;
                if take {
                    if !out.is_empty() {
                        out.push(String::new());
                    }
                    out.push(format!("Version {ver}"));
                }
                continue;
            }
        }
        if take && !l.trim().is_empty() {
            let t = l.trim_start();
            let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).map(|r| format!("\u{2022} {}", r.trim_start())).unwrap_or_else(|| l.to_string());
            out.push(t);
        }
    }
    out.join("\n")
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpdateOffer {
    pub from: String,
    pub to: String,
    pub changes: String,
    /// An earlier update is unfinished: only the rebuild is left (no download).
    pub finish: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Checking,
    UpToDate(String),
    Available(UpdateOffer),
    /// Could not reach the server (offline): no banner.
    Offline,
}

/// What the Play screen shows: the offer for (local, built) versions against the remote text. `remote_version` None = offline.
pub fn evaluate(local: &str, built: &str, remote_version: Option<&str>, changelog: Option<&str>) -> UpdateState {
    let mut state = UpdateState::Offline;
    if let Some(rv) = remote_version.map(|s| s.trim()).filter(|v| is_version(v)) {
        use std::cmp::Ordering::*;
        state = UpdateState::UpToDate(local.to_string());
        if compare_version(rv, local) == Greater {
            let from = if !built.is_empty() && compare_version(built, local) == Less { built } else { local };
            let changes = changelog.map(|c| changelog_for(c, from, rv)).filter(|t| !t.is_empty()).unwrap_or_else(|| "(The list of changes could not be loaded.)".to_string());
            return UpdateState::Available(UpdateOffer { from: from.to_string(), to: rv.to_string(), changes, finish: false });
        }
    }
    // the files are newer than the build (an unfinished update): offer the rebuild, with the local change list
    if !built.is_empty() && built != local && compare_version(built, local) == std::cmp::Ordering::Less {
        let changes = changelog.map(|c| changelog_for(c, built, local)).filter(|t| !t.is_empty()).unwrap_or_default();
        return UpdateState::Available(UpdateOffer { from: built.to_string(), to: local.to_string(), changes, finish: true });
    }
    state
}

/// `evaluate` for one game's feed. A game other than R&C 1 whose repo or VERSION does not exist yet (404) is silent: "up to date", no
/// "Could not check" line (an unfinished rebuild is still offered). A real failure (offline) stays `Offline`.
pub fn evaluate_feed(game: &GameDef, local: &str, built: &str, remote: &Result<String, Fetch>, changelog: Option<&str>) -> UpdateState {
    let st = evaluate(local, built, remote.as_deref().ok(), changelog);
    if st == UpdateState::Offline && game.id != "rac1" && *remote == Err(Fetch::Missing) {
        return UpdateState::UpToDate(local.to_string());
    }
    st
}

/// Runs the check on a thread against the feed of `game` (its own repo); the result lands in `slot`.
pub fn check_async(game: &'static GameDef, local: String, built: String, base: Option<String>, root: PathBuf, slot: Arc<Mutex<UpdateState>>) {
    std::thread::spawn(move || {
        let t = Duration::from_millis(4000);
        let rv = fetch_remote(base.as_deref(), game.repo, "VERSION", t);
        let newer = rv.as_deref().map(|v| is_version(v.trim()) && compare_version(v.trim(), &local) == std::cmp::Ordering::Greater).unwrap_or(false);
        let log = if newer { fetch_remote(base.as_deref(), game.repo, "CHANGELOG.md", t).ok() } else { std::fs::read_to_string(root.join("CHANGELOG.md")).ok() };
        let st = evaluate_feed(game, &local, &built, &rv, log.as_deref());
        if let Ok(mut g) = slot.lock() {
            *g = st;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    const LOG: &str = "# Changelog\n\n## v0.4.0\n- Fixed the thing\n- Second fix\n\n## v0.3.1\n* AMD start-up fix\n\n## v0.3.0\n- Big release\n";

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_version("0.3.10", "0.3.9"), Ordering::Greater);
        assert_eq!(compare_version("0.3.1", "0.3.1"), Ordering::Equal);
        assert_eq!(compare_version("0.2.9", "0.3.0"), Ordering::Less);
    }

    #[test]
    fn change_list_is_the_versions_between() {
        let t = changelog_for(LOG, "0.3.0", "0.4.0");
        assert_eq!(t, "Version 0.4.0\n\u{2022} Fixed the thing\n\u{2022} Second fix\n\nVersion 0.3.1\n\u{2022} AMD start-up fix");
        assert_eq!(changelog_for(LOG, "0.3.1", "0.3.1"), "");
        assert_eq!(changelog_for(LOG, "0.3.1", "0.4.0"), "Version 0.4.0\n\u{2022} Fixed the thing\n\u{2022} Second fix");
    }

    #[test]
    fn offer_when_the_remote_is_newer() {
        match evaluate("0.3.1", "0.3.1", Some("0.4.0\n"), Some(LOG)) {
            UpdateState::Available(o) => {
                assert_eq!((o.from.as_str(), o.to.as_str(), o.finish), ("0.3.1", "0.4.0", false));
                assert!(o.changes.starts_with("Version 0.4.0"));
            }
            s => panic!("{s:?}"),
        }
        assert_eq!(evaluate("0.4.0", "0.4.0", Some("0.4.0"), Some(LOG)), UpdateState::UpToDate("0.4.0".into()));
        assert_eq!(evaluate("0.4.0", "0.4.0", None, None), UpdateState::Offline);
        assert_eq!(evaluate("0.4.0", "0.4.0", Some("garbage"), None), UpdateState::Offline);
    }

    #[test]
    fn unfinished_update_uses_the_built_version_as_start() {
        // files are 0.4.0 but the game exe is still 0.3.0, the server says 0.3.6: from = built
        match evaluate("0.4.0", "0.3.0", Some("0.3.6"), Some("## v0.3.6\n- x\n## v0.4.0\n- y\n## v0.3.1\n- z\n")) {
            UpdateState::Available(o) => assert_eq!(o.from, "0.3.0"),
            s => panic!("{s:?}"),
        }
        // offline: the local rebuild is still offered
        match evaluate("0.4.0", "0.3.0", None, Some(LOG)) {
            UpdateState::Available(o) => assert!(o.finish && o.to == "0.4.0"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn local_bases_are_told_from_urls() {
        assert_eq!(local_base("https://example.com/x"), None);
        assert_eq!(local_base("http://localhost:8000"), None);
        assert_eq!(local_base("file:///E:/base/x"), Some(PathBuf::from("E:/base/x")));
        assert_eq!(local_base("file:///tmp/base"), Some(PathBuf::from("/tmp/base")));
        assert_eq!(local_base("/home/p/base"), Some(PathBuf::from("/home/p/base")));
        assert_eq!(local_base(" D:\\base "), Some(PathBuf::from("D:\\base")));
    }

    #[test]
    fn test_base_folder_and_file_url() {
        let d = std::env::temp_dir().join(format!("racl_upd_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("VERSION"), "9.9.9\n").unwrap();
        let t = Duration::from_millis(100);
        assert_eq!(read_remote(Some(d.to_str().unwrap()), "VERSION", t).as_deref(), Some("9.9.9\n"));
        let url = format!("file:///{}", d.to_str().unwrap().replace('\\', "/"));
        assert_eq!(read_remote(Some(&url), "VERSION", t).as_deref(), Some("9.9.9\n"));
        assert_eq!(read_remote(Some(d.to_str().unwrap()), "NOPE", t), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A local HTTP server answering `n` requests with `status`; returns its base URL and the requested paths.
    fn serve(status: &str, body: &'static str, n: usize) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Write};
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let status = status.to_string();
        std::thread::spawn(move || {
            for _ in 0..n {
                let Ok((mut c, _)) = l.accept() else { return };
                let mut line = String::new();
                let mut r = BufReader::new(c.try_clone().unwrap());
                r.read_line(&mut line).ok();
                let _ = tx.send(line.split_whitespace().nth(1).unwrap_or("").to_string());
                let _ = write!(c, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (base, rx)
    }

    #[test]
    fn each_game_reads_its_own_repo() {
        let (r1, r2) = (crate::games::game_by_id("rac1").unwrap(), crate::games::game_by_id("rac2").unwrap());
        assert_eq!(raw_base(r1.repo), "https://raw.githubusercontent.com/AsobiCodes/rac-rs/main");
        assert_eq!(raw_base(r2.repo), "https://raw.githubusercontent.com/AsobiCodes/rac-rs2/main");
        assert_eq!(raw_base(r2.repo), raw_base(crate::releases::RAC2_REPO));
    }

    #[test]
    fn a_404_is_missing_and_a_dead_server_is_failed() {
        let t = Duration::from_millis(1500);
        let (base, rx) = serve("404 Not Found", "nope", 1);
        assert_eq!(fetch_remote(Some(&base), crate::releases::RAC2_REPO, "VERSION", t), Err(Fetch::Missing));
        assert_eq!(rx.recv().unwrap(), "/VERSION");
        let (base, _rx) = serve("500 Oops", "x", 1);
        assert_eq!(fetch_remote(Some(&base), crate::releases::RAC2_REPO, "VERSION", t), Err(Fetch::Failed));
        let (base, _rx) = serve("200 OK", "0.1.1\n", 1);
        assert_eq!(fetch_remote(Some(&base), crate::releases::RAC2_REPO, "VERSION", t).as_deref(), Ok("0.1.1\n"));
        assert_eq!(fetch_remote(Some("http://127.0.0.1:1"), crate::releases::RAC2_REPO, "VERSION", t), Err(Fetch::Failed));
    }

    #[test]
    fn feed_rules_per_game() {
        let (r1, r2) = (crate::games::game_by_id("rac1").unwrap(), crate::games::game_by_id("rac2").unwrap());
        let ok = |v: &str| Ok::<String, Fetch>(v.to_string());
        // R&C 2: older than the newest release -> offer; same -> none; 404 -> silent
        assert!(matches!(evaluate_feed(r2, "0.1.0", "0.1.0", &ok("0.1.1\n"), Some("## v0.1.1\n- a\n")), UpdateState::Available(o) if o.to == "0.1.1"));
        assert_eq!(evaluate_feed(r2, "0.1.1", "0.1.1", &ok("0.1.1"), None), UpdateState::UpToDate("0.1.1".into()));
        assert_eq!(evaluate_feed(r2, "0.1.0", "0.1.0", &Err(Fetch::Missing), None), UpdateState::UpToDate("0.1.0".into()));
        assert_eq!(evaluate_feed(r2, "0.1.0", "0.1.0", &Err(Fetch::Failed), None), UpdateState::Offline);
        // an unfinished rebuild is still offered when the feed is missing
        assert!(matches!(evaluate_feed(r2, "0.1.1", "0.1.0", &Err(Fetch::Missing), None), UpdateState::Available(o) if o.finish));
        // R&C 1 keeps its old behaviour
        assert_eq!(evaluate_feed(r1, "0.5.1", "0.5.1", &Err(Fetch::Missing), None), UpdateState::Offline);
    }
}
