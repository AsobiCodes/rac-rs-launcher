use super::*;
use std::cell::Cell;

const WIN: bool = cfg!(windows);

fn scratch(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rac_lu_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A file that passes `check_head` for this system, with one distinguishing byte at the end.
fn fake_lib(win: bool, tag: u8) -> Vec<u8> {
    let mut b = vec![0u8; 0x100];
    if win {
        b[..2].copy_from_slice(b"MZ");
        b[0x3C] = 0x80;
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
    } else {
        b[..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        b[4] = 2;
        b[5] = 1;
        b[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
    }
    b.push(tag);
    b
}

fn hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|x| format!("{x:02x}")).collect()
}

fn file_name(win: bool) -> &'static str {
    if win { "omniloader-windows-x86_64.dll" } else { "libomniloader-linux-x86_64.so" }
}

fn manifest_text(version: &str, abi: &str, min_host: &str, lib: &[u8], claimed_sha: Option<String>) -> String {
    let key = platform_key(WIN, "x86_64").unwrap();
    serde_json::json!({
        "format": 1, "name": "omniloader", "version": version, "abi": abi, "min_host_abi": min_host,
        "files": { key: { "name": file_name(WIN), "install_as": loader::lib_name_for(WIN), "size": lib.len(), "sha256": claimed_sha.unwrap_or_else(|| hex(lib)) } }
    })
    .to_string()
}

/// A release folder (the test source of `releases.rs`) with the manifest and the library.
fn release(dir: &Path, version: &str, abi: &str, min_host: &str, lib: &[u8], claimed_sha: Option<String>, lib_digest: Option<String>) {
    let man = manifest_text(version, abi, min_host, lib, claimed_sha);
    let lib_digest_owned = lib_digest;
    releases::write_test_release(dir, &format!("v{version}"), "notes", &[(MANIFEST_ASSET, man.as_bytes()), (file_name(WIN), lib)], &|name, h| {
        if name == file_name(WIN) { lib_digest_owned.clone().or(Some(format!("sha256:{h}"))) } else { Some(format!("sha256:{h}")) }
    });
}

fn game(d: &Path, lib: Option<(&[u8], Option<&str>)>) -> PathBuf {
    let exe = d.join(if WIN { "rac-rs.exe" } else { "rac-rs" });
    std::fs::write(&exe, b"x").unwrap();
    if let Some((bytes, side)) = lib {
        let l = loader::lib_path(&exe);
        std::fs::write(&l, bytes).unwrap();
        if let Some(s) = side {
            std::fs::write(with_suffix(&l, ".json"), s).unwrap();
        }
    }
    exe
}

fn host(minor: u32) -> Abi {
    Abi { major: 1, minor }
}

fn verdict_for(dir: &Path, h: Abi, inst: &Installed) -> Verdict {
    let src = ReleaseSource::Folder(dir.to_path_buf());
    let (rel, man) = fetch(&src, &dir.join("tmp")).unwrap();
    evaluate(&rel, &man, h, inst, WIN, "x86_64")
}

fn unknown_installed(lib: &[u8]) -> Installed {
    Installed { present: true, version: None, abi: None, sha256: Some(hex(lib)) }
}

#[test]
fn abi_text_round_trips_and_orders() {
    assert_eq!(Abi::parse("1.6"), Some(Abi { major: 1, minor: 6 }));
    assert_eq!(Abi::parse(" 2.0 ").map(|a| a.text()), Some("2.0".to_string()));
    for bad in ["", "1", "1.6.2", "a.b", "1.", ".6", "-1.2"] {
        assert_eq!(Abi::parse(bad), None, "{bad}");
    }
    assert!(Abi { major: 1, minor: 7 } > Abi { major: 1, minor: 6 } && Abi { major: 2, minor: 0 } > Abi { major: 1, minor: 99 });
}

#[test]
fn the_host_line_is_found_among_other_output() {
    assert_eq!(parse_host_output(r#"{"omni_host":{"abi":"1.6","size":360,"lib":"omniloader.dll"}}"#), Some(Abi { major: 1, minor: 6 }));
    assert_eq!(parse_host_output("noise\r\n{\"omni_host\":{\"abi\":\"1.9\"}}\r\n"), Some(Abi { major: 1, minor: 9 }));
    for bad in ["", "usage: rac-rs ...", "{}", r#"{"omni_host":{"abi":7}}"#, r#"{"omni_host":{"abi":"x"}}"#] {
        assert_eq!(parse_host_output(bad), None, "{bad}");
    }
}

#[test]
fn an_old_game_is_never_run_and_a_new_one_has_to_answer() {
    let exe = Path::new("rac-rs");
    let boom = |_: &Path| -> Option<String> { panic!("an old game must not be started") };
    for v in ["0.4.1", "0.4.0", "0.2.1", "0.1.0", "", "garbage"] {
        assert_eq!(host_abi_with(exe, v, &boom), Err(OLD_GAME.to_string()), "{v:?}");
        assert!(!supported_game(v), "{v:?}");
    }
    for v in ["0.4.2", "0.4.3", "0.5.0", "1.0.0", "v0.4.2"] {
        assert!(supported_game(v), "{v}");
    }
    let says = |_: &Path| Some(r#"{"omni_host":{"abi":"1.8"}}"#.to_string());
    assert_eq!(host_abi_with(exe, "0.4.2", &says), Ok(Abi { major: 1, minor: 8 }));
    assert_eq!(host_abi_with(exe, "0.5.0", &says), Ok(Abi { major: 1, minor: 8 }));
    // a game that is new enough but does not answer (or prints junk) is not guessed about: no ABI, a plain reason
    let silent = |_: &Path| None;
    assert_eq!(host_abi_with(exe, "0.4.2", &silent), Err(COULD_NOT_ASK.to_string()));
    let junk = |_: &Path| Some("hello".to_string());
    assert_eq!(host_abi_with(exe, "0.4.2", &junk), Err(COULD_NOT_ASK.to_string()));
    assert!(COULD_NOT_ASK.contains("no update is offered") && OLD_GAME.contains("0.4.2"));
}

#[test]
fn ask_exe_reads_what_a_program_prints_and_ignores_a_missing_one() {
    let d = scratch("ask");
    assert_eq!(ask_exe(&d.join("none"), Duration::from_secs(2)), None);
    let line = r#"{"omni_host":{"abi":"1.6","size":360}}"#;
    #[cfg(windows)]
    let exe = {
        let p = d.join("fake.cmd");
        std::fs::write(&p, format!("@echo {line}\r\n")).unwrap();
        p
    };
    #[cfg(unix)]
    let exe = {
        use std::os::unix::fs::PermissionsExt;
        let p = d.join("fake.sh");
        std::fs::write(&p, format!("#!/bin/sh\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let out = ask_exe(&exe, Duration::from_secs(10)).expect("the program answered");
    assert_eq!(parse_host_output(&out), Some(Abi { major: 1, minor: 6 }));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_manifest_is_read_strictly() {
    let lib = fake_lib(WIN, 1);
    let m = parse_manifest(&manifest_text("0.2.0", "1.7", "1.4", &lib, None)).unwrap();
    assert_eq!((m.version.as_str(), m.abi, m.min_host), ("0.2.0", Abi { major: 1, minor: 7 }, Abi { major: 1, minor: 4 }));
    assert_eq!(m.files[platform_key(WIN, "x86_64").unwrap()].sha256, hex(&lib));
    // min_host_abi is optional
    let no_min = manifest_text("0.2.0", "1.7", "1.0", &lib, None).replace("\"min_host_abi\":\"1.0\",", "");
    assert_eq!(parse_manifest(&no_min).unwrap().min_host, Abi { major: 1, minor: 0 });
    for (what, text) in [
        ("not json", "{".to_string()),
        ("format 2", manifest_text("0.2.0", "1.7", "1.0", &lib, None).replace("\"format\":1", "\"format\":2")),
        ("bad version", manifest_text("x", "1.7", "1.0", &lib, None)),
        ("bad abi", manifest_text("0.2.0", "one", "1.0", &lib, None)),
        ("short sha", manifest_text("0.2.0", "1.7", "1.0", &lib, Some("abcd".into()))),
        ("non-hex sha", manifest_text("0.2.0", "1.7", "1.0", &lib, Some("z".repeat(64)))),
        ("path in name", manifest_text("0.2.0", "1.7", "1.0", &lib, None).replace(file_name(WIN), "../evil.dll")),
        ("path in install_as", manifest_text("0.2.0", "1.7", "1.0", &lib, None).replace(loader::lib_name_for(WIN), "sub/omniloader.dll")),
    ] {
        assert!(parse_manifest(&text).is_err(), "{what} must be refused");
    }
}

#[test]
fn a_newer_loader_is_offered_and_the_same_or_older_one_is_not() {
    let d = scratch("verdict");
    let lib = fake_lib(WIN, 2);
    release(&d, "0.2.0", "1.7", "1.0", &lib, None, None);
    let old = fake_lib(WIN, 1);
    // unknown installed file: taken for 0.1.0
    match verdict_for(&d, host(6), &unknown_installed(&old)) {
        Verdict::Offer(o) => assert_eq!((o.version.as_str(), o.abi, o.tag.as_str()), ("0.2.0", Abi { major: 1, minor: 7 }, "v0.2.0")),
        v => panic!("{v:?}"),
    }
    // nothing installed at all: offered too
    assert!(matches!(verdict_for(&d, host(6), &Installed::default()), Verdict::Offer(_)));
    // the same file
    assert_eq!(verdict_for(&d, host(6), &unknown_installed(&lib)), Verdict::UpToDate("0.2.0".into()));
    // a known newer or equal version with another hash
    let mut inst = unknown_installed(&old);
    inst.version = Some("0.2.0".into());
    assert!(matches!(verdict_for(&d, host(6), &inst), Verdict::UpToDate(_)));
    inst.version = Some("0.3.1".into());
    assert!(matches!(verdict_for(&d, host(6), &inst), Verdict::UpToDate(_)));
    inst.version = Some("0.1.5".into());
    assert!(matches!(verdict_for(&d, host(6), &inst), Verdict::Offer(_)));
    // never to a lower ABI minor than the installed one
    inst.abi = Some(Abi { major: 1, minor: 9 });
    assert!(matches!(verdict_for(&d, host(6), &inst), Verdict::UpToDate(_)));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_release_for_0_1_0_is_not_offered_over_an_unknown_file() {
    let d = scratch("first");
    release(&d, "0.1.0", "1.6", "1.0", &fake_lib(WIN, 3), None, None);
    assert!(matches!(verdict_for(&d, host(6), &unknown_installed(&fake_lib(WIN, 9))), Verdict::UpToDate(_)));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_abi_the_game_cannot_serve_is_blocked_with_a_reason() {
    let d = scratch("abi");
    let old = unknown_installed(&fake_lib(WIN, 1));
    // another major, newer and older
    release(&d, "2.0.0", "2.0", "2.0", &fake_lib(WIN, 2), None, None);
    let Verdict::Blocked(t) = verdict_for(&d, host(6), &old) else { panic!() };
    assert!(t.contains("newer game") && t.contains("2.0") && t.contains("1.6"), "{t}");
    release(&d, "0.9.0", "0.9", "0.0", &fake_lib(WIN, 2), None, None);
    let Verdict::Blocked(t) = verdict_for(&d, host(6), &old) else { panic!() };
    assert!(t.contains("older game"), "{t}");
    // same major, but the loader needs a host that serves more than this game does
    release(&d, "0.3.0", "1.8", "1.7", &fake_lib(WIN, 2), None, None);
    let Verdict::Blocked(t) = verdict_for(&d, host(6), &old) else { panic!() };
    assert!(t.contains("1.7 or newer") && t.contains("serves 1.6"), "{t}");
    // a newer minor than the host is fine when min_host allows it
    release(&d, "0.3.0", "1.8", "1.0", &fake_lib(WIN, 2), None, None);
    assert!(matches!(verdict_for(&d, host(6), &old), Verdict::Offer(_)));
    // no file for this CPU
    let src = ReleaseSource::Folder(d.clone());
    let (rel, man) = fetch(&src, &d.join("tmp")).unwrap();
    assert!(matches!(evaluate(&rel, &man, host(6), &old, WIN, "aarch64"), Verdict::Blocked(_)));
    // tag and manifest disagree
    let mut rel2 = rel.clone();
    rel2.tag_name = "v0.3.1".into();
    assert!(matches!(evaluate(&rel2, &man, host(6), &old, WIN, "x86_64"), Verdict::Blocked(_)));
    let _ = std::fs::remove_dir_all(&d);
}

fn offer_of(d: &Path) -> Offer {
    match verdict_for(d, host(6), &Installed::default()) {
        Verdict::Offer(o) => o,
        v => panic!("{v:?}"),
    }
}

#[test]
fn an_install_replaces_the_file_keeps_the_old_one_and_writes_the_sidecar() {
    let d = scratch("install");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, Some(&format!(r#"{{"version":"0.1.0","abi":"1.6","sha256":"{}"}}"#, hex(&old))))));
    let src = ReleaseSource::Folder(d.join("rel"));
    install(&src, &offer_of(&d.join("rel")), &exe, &Transfer::default(), &|| false, WIN).unwrap();
    let lib = loader::lib_path(&exe);
    assert_eq!(std::fs::read(&lib).unwrap(), new);
    assert_eq!(std::fs::read(with_suffix(&lib, ".prev")).unwrap(), old);
    assert!(std::fs::read_to_string(with_suffix(&lib, ".json.prev")).unwrap().contains("0.1.0"));
    assert!(!with_suffix(&lib, ".new").exists());
    let i = installed(&exe);
    assert_eq!((i.present, i.version.as_deref(), i.abi), (true, Some("0.2.0"), Some(Abi { major: 1, minor: 7 })));
    assert_eq!(i.sha256, Some(hex(&new)));
    assert!(matches!(verdict_for(&d.join("rel"), host(6), &i), Verdict::UpToDate(_)));
    // the install does not need an old file either
    let g2 = d.join("game2");
    std::fs::create_dir_all(&g2).unwrap();
    let exe2 = game(&g2, None);
    install(&src, &offer_of(&d.join("rel")), &exe2, &Transfer::default(), &|| false, WIN).unwrap();
    assert_eq!(std::fs::read(loader::lib_path(&exe2)).unwrap(), new);
    assert!(!can_roll_back(&exe2));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn nothing_changes_while_the_game_runs_or_when_it_starts_during_the_download() {
    let d = scratch("running");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, None)));
    let lib = loader::lib_path(&exe);
    let src = ReleaseSource::Folder(d.join("rel"));
    let offer = offer_of(&d.join("rel"));
    assert_eq!(install(&src, &offer, &exe, &Transfer::default(), &|| true, WIN), Err(GAME_RUNNING.to_string()));
    // the game starts after the download: asked a second time
    let n = Cell::new(0);
    let late = || {
        n.set(n.get() + 1);
        n.get() > 1
    };
    assert_eq!(install(&src, &offer, &exe, &Transfer::default(), &late, WIN), Err(GAME_RUNNING.to_string()));
    for extra in [".new", ".prev", ".json", ".json.prev"] {
        assert!(!with_suffix(&lib, extra).exists(), "{extra}");
    }
    assert_eq!(std::fs::read(&lib).unwrap(), old);
    assert_eq!(roll_back(&exe, &|| true), Err(GAME_RUNNING.to_string()));
    assert_eq!(restore_bundled(&d, &exe, &|| true, WIN), Err("This game folder has no bundled mod loader to go back to.".to_string()));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_damaged_wrong_or_disagreeing_download_leaves_the_installed_file_alone() {
    let d = scratch("bad");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, None)));
    let lib = loader::lib_path(&exe);
    let untouched = |why: &str| {
        assert_eq!(std::fs::read(&lib).unwrap(), old, "{why}");
        for extra in [".new", ".prev", ".json", ".part"] {
            assert!(!with_suffix(&lib, extra).exists(), "{why}: {extra}");
        }
    };
    let try_install = |rel: &Path| {
        let src = ReleaseSource::Folder(rel.to_path_buf());
        install(&src, &offer_of(rel), &exe, &Transfer::default(), &|| false, WIN)
    };
    // the manifest says another hash than the file has
    release(&d.join("r1"), "0.2.0", "1.7", "1.0", &new, Some(hex(b"something else")), None);
    assert!(try_install(&d.join("r1")).unwrap_err().contains("checksum"));
    untouched("manifest sha");
    // GitHub's digest and the manifest disagree
    release(&d.join("r2"), "0.2.0", "1.7", "1.0", &new, None, Some(format!("sha256:{}", hex(b"other"))));
    assert!(try_install(&d.join("r2")).unwrap_err().contains("disagree"));
    untouched("two digests");
    // right hash, but not a library for this system
    let junk = b"this is not a library".to_vec();
    release(&d.join("r3"), "0.2.0", "1.7", "1.0", &junk, None, None);
    assert!(try_install(&d.join("r3")).unwrap_err().contains("not a 64-bit"));
    untouched("header");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_release_without_a_checksum_on_its_manifest_is_refused() {
    let d = scratch("nodigest");
    let lib = fake_lib(WIN, 2);
    let man = manifest_text("0.2.0", "1.7", "1.0", &lib, None);
    releases::write_test_release(&d, "v0.2.0", "", &[(MANIFEST_ASSET, man.as_bytes()), (file_name(WIN), &lib)], &|_, _| None);
    assert_eq!(fetch(&ReleaseSource::Folder(d.clone()), &d.join("tmp")).unwrap_err(), ReleaseError::NoDigest);
    // and a release without the manifest
    let d2 = scratch("nomanifest");
    releases::write_test_release(&d2, "v0.2.0", "", &[(file_name(WIN), &lib)], &|_, h| Some(format!("sha256:{h}")));
    assert!(matches!(fetch(&ReleaseSource::Folder(d2.clone()), &d2.join("tmp")), Err(ReleaseError::Bad(_))));
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&d2);
}

#[test]
fn rolling_back_swaps_the_two_loaders_and_their_sidecars_both_ways() {
    let d = scratch("rollback");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, Some(&format!(r#"{{"version":"0.1.0","abi":"1.6","sha256":"{}"}}"#, hex(&old))))));
    let src = ReleaseSource::Folder(d.join("rel"));
    install(&src, &offer_of(&d.join("rel")), &exe, &Transfer::default(), &|| false, WIN).unwrap();
    let lib = loader::lib_path(&exe);
    assert!(can_roll_back(&exe));
    roll_back(&exe, &|| false).unwrap();
    assert_eq!(std::fs::read(&lib).unwrap(), old);
    assert_eq!(installed(&exe).version.as_deref(), Some("0.1.0"));
    roll_back(&exe, &|| false).unwrap();
    assert_eq!(std::fs::read(&lib).unwrap(), new);
    assert_eq!(installed(&exe).version.as_deref(), Some("0.2.0"));
    let _ = std::fs::remove_dir_all(&d);
    let d2 = scratch("rollback_none");
    let exe2 = game(&d2, Some((&old, None)));
    assert!(roll_back(&exe2, &|| false).unwrap_err().contains("no previous"));
    let _ = std::fs::remove_dir_all(&d2);
}

#[test]
fn the_games_own_loader_can_be_put_back_and_the_update_kept_for_rollback() {
    let d = scratch("bundled");
    let (bundled, newer) = (fake_lib(WIN, 1), fake_lib(WIN, 5));
    std::fs::create_dir_all(d.join("bundled")).unwrap();
    std::fs::write(bundled_lib(&d, WIN), &bundled).unwrap();
    std::fs::write(with_suffix(&bundled_lib(&d, WIN), ".json"), format!(r#"{{"version":"0.1.0","abi":"1.6","sha256":"{}"}}"#, hex(&bundled))).unwrap();
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&newer, Some(&format!(r#"{{"version":"0.5.0","abi":"1.6","sha256":"{}"}}"#, hex(&newer))))));
    let lib = loader::lib_path(&exe);
    assert_eq!(restore_bundled(&d, &exe, &|| false, WIN), Ok(true));
    assert_eq!(std::fs::read(&lib).unwrap(), bundled);
    assert_eq!(installed(&exe).version.as_deref(), Some("0.1.0"));
    assert_eq!(std::fs::read(with_suffix(&lib, ".prev")).unwrap(), newer);
    // already the bundled one
    assert_eq!(restore_bundled(&d, &exe, &|| false, WIN), Ok(false));
    assert_eq!(std::fs::read(with_suffix(&lib, ".prev")).unwrap(), newer);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_sidecar_that_names_another_file_is_stale() {
    let d = scratch("stale");
    let lib = fake_lib(WIN, 1);
    let exe = game(&d, Some((&lib, Some(&format!(r#"{{"version":"0.9.0","abi":"1.6","sha256":"{}"}}"#, hex(b"some other file"))))));
    let i = installed(&exe);
    assert_eq!((i.present, i.version, i.abi, i.sha256), (true, None, None, Some(hex(&lib))));
    // one that names this file is believed
    std::fs::write(with_suffix(&loader::lib_path(&exe), ".json"), format!(r#"{{"version":"0.9.0","abi":"1.6","sha256":"{}"}}"#, hex(&lib).to_uppercase())).unwrap();
    assert_eq!(installed(&exe).version.as_deref(), Some("0.9.0"));
    assert_eq!(installed(&d.join("no_such_folder").join("no_such_exe")), Installed::default());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn headers_are_checked_for_each_system() {
    assert!(check_head(true, &fake_lib(true, 0)).is_ok());
    assert!(check_head(false, &fake_lib(false, 0)).is_ok());
    assert!(check_head(true, &fake_lib(false, 0)).is_err() && check_head(false, &fake_lib(true, 0)).is_err());
    let mut arm = fake_lib(false, 0);
    arm[18] = 0xB7;
    assert!(check_head(false, &arm).is_err(), "an ARM library is refused");
    let mut x86 = fake_lib(true, 0);
    x86[0x84] = 0x4C;
    x86[0x85] = 0x01;
    assert!(check_head(true, &x86).is_err(), "a 32-bit DLL is refused");
    assert!(check_head(true, b"").is_err() && check_head(false, b"MZ").is_err());
}

#[test]
fn platform_keys_exist_only_for_x86_64() {
    assert_eq!(platform_key(true, "x86_64"), Some("windows-x86_64"));
    assert_eq!(platform_key(false, "x86_64"), Some("linux-x86_64"));
    assert_eq!(platform_key(false, "aarch64"), None);
    assert_eq!(platform_key(true, "aarch64"), None);
}

// ---------------------------------------------------------------------------------------------- the launcher's own question is not "game running"

#[test]
fn the_launchers_own_question_to_the_game_is_flagged_while_it_runs() {
    let d = scratch("probe");
    let line = r#"{"omni_host":{"abi":"1.6","size":360}}"#;
    #[cfg(windows)]
    let exe = {
        let p = d.join("slow.cmd");
        std::fs::write(&p, format!("@echo off\r\nping -n 3 127.0.0.1 >nul\r\necho {line}\r\n")).unwrap();
        p
    };
    #[cfg(unix)]
    let exe = {
        use std::os::unix::fs::PermissionsExt;
        let p = d.join("slow.sh");
        std::fs::write(&p, format!("#!/bin/sh\nsleep 2\necho '{line}'\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let worker = {
        let exe = exe.clone();
        std::thread::spawn(move || ask_exe(&exe, Duration::from_secs(30)))
    };
    // the program needs about 2 s: half a second in, the watch for "is the game running?" has to be told to look away
    std::thread::sleep(Duration::from_millis(500));
    assert!(probing(), "the probe is not flagged while the program runs");
    assert!(!worker.is_finished());
    let out = worker.join().unwrap().expect("the program answered");
    assert_eq!(parse_host_output(&out), Some(Abi { major: 1, minor: 6 }));
    // the flag stays up for a moment after the program ended (the system lets go of the program file a beat later)
    assert!(probing());
    let _ = std::fs::remove_dir_all(&d);
}

/// The in-use check with a made-up clock: `probing` is true for the first `probe_calls` looks, `in_use` is true while it is (the probe's own
/// process holds the file) or when `really_running`. Returns (answer, how long it waited in total).
fn settled_run(probe_calls: u32, really_running: bool, limit_ms: u64) -> (bool, Duration) {
    let looks = Cell::new(0u32);
    let slept = Cell::new(Duration::ZERO);
    let probing = || {
        looks.set(looks.get() + 1);
        looks.get() <= probe_calls
    };
    let in_use = || really_running || looks.get() <= probe_calls;
    let sleep = |d: Duration| slept.set(slept.get() + d);
    let a = in_use_after_probe(&probing, &in_use, Duration::from_millis(limit_ms), Duration::from_millis(50), &sleep);
    (a, slept.get())
}

#[test]
fn the_in_use_check_waits_for_the_launchers_own_question_and_not_longer() {
    // no question running: no wait, the plain answer either way
    assert_eq!(settled_run(0, false, 7_000), (false, Duration::ZERO));
    assert_eq!(settled_run(0, true, 7_000), (true, Duration::ZERO));
    // a question that ends after 3 looks: waited exactly 3 steps, and the file no longer counts as in use
    assert_eq!(settled_run(3, false, 7_000), (false, Duration::from_millis(150)));
    // the game really runs: still refused after the question ended
    assert_eq!(settled_run(3, true, 7_000), (true, Duration::from_millis(150)));
    // a question that never ends is waited for at most the limit, then the plain answer (in use: the safe side)
    assert_eq!(settled_run(u32::MAX, false, 1_000), (true, Duration::from_millis(1_000)));
    // the wait covers the longest question (its own limit) plus the tail after it
    assert!(PROBE_WAIT >= Duration::from_millis(ASK_LIMIT_MS + PROBE_TAIL_MS));
}

// ---------------------------------------------------------------------------------------------- sizes, caps and addresses

/// A release whose manifest lists `manifest_size` for the library (the release's own asset size is the real one).
fn release_sized(dir: &Path, version: &str, lib: &[u8], manifest_size: u64) {
    let key = platform_key(WIN, "x86_64").unwrap();
    let man = serde_json::json!({
        "format": 1, "name": "omniloader", "version": version, "abi": "1.7", "min_host_abi": "1.0",
        "files": { key: { "name": file_name(WIN), "install_as": loader::lib_name_for(WIN), "size": manifest_size, "sha256": hex(lib) } }
    })
    .to_string();
    releases::write_test_release(dir, &format!("v{version}"), "notes", &[(MANIFEST_ASSET, man.as_bytes()), (file_name(WIN), lib)], &|_, h| Some(format!("sha256:{h}")));
}

#[test]
fn a_size_that_does_not_add_up_is_blocked() {
    let d = scratch("sizes");
    let lib = fake_lib(WIN, 2);
    let old = unknown_installed(&fake_lib(WIN, 1));
    let n = lib.len() as u64;
    release_sized(&d, "0.2.0", &lib, n);
    assert!(matches!(verdict_for(&d, host(6), &old), Verdict::Offer(_)));
    for wrong in [0, n + 1, n - 1] {
        release_sized(&d, "0.2.0", &lib, wrong);
        let Verdict::Blocked(t) = verdict_for(&d, host(6), &old) else { panic!("size {wrong} was accepted") };
        assert!(t.contains("size"), "{t}");
    }
    // too big for a loader even when manifest and release agree
    release_sized(&d, "0.2.0", &lib, n);
    let src = ReleaseSource::Folder(d.clone());
    let (mut rel, mut man) = fetch(&src, &d.join("tmp")).unwrap();
    for a in rel.assets.iter_mut().filter(|a| a.name == file_name(WIN)) {
        a.size = LIB_MAX + 1;
    }
    man.files.values_mut().for_each(|f| f.size = LIB_MAX + 1);
    assert!(matches!(evaluate(&rel, &man, host(6), &old, WIN, "x86_64"), Verdict::Blocked(t) if t.contains("size")));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn install_refuses_a_wrong_size_and_stops_a_body_bigger_than_listed() {
    let d = scratch("body");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, None)));
    let lib = loader::lib_path(&exe);
    let src = ReleaseSource::Folder(d.join("rel"));
    let good = offer_of(&d.join("rel"));
    let untouched = |why: &str| {
        assert_eq!(std::fs::read(&lib).unwrap(), old, "{why}");
        for extra in [".new", ".new.part", ".prev", ".json"] {
            assert!(!with_suffix(&lib, extra).exists(), "{why}: {extra}");
        }
    };
    // the two sizes disagree, or are missing
    for (file_size, asset_size) in [(good.asset.size + 1, good.asset.size), (0, 0), (5, 6)] {
        let mut o = good.clone();
        o.file.size = file_size;
        o.asset.size = asset_size;
        let e = install(&src, &o, &exe, &Transfer::default(), &|| false, WIN).unwrap_err();
        assert!(e.contains("size"), "{e}");
        untouched("sizes");
    }
    // both say 10 bytes, the file has more: the body is stopped, nothing is kept
    let mut o = good.clone();
    o.file.size = 10;
    o.asset.size = 10;
    let t = Transfer::default();
    let e = install(&src, &o, &exe, &t, &|| false, WIN).unwrap_err();
    assert!(e.contains("larger than the release says"), "{e}");
    untouched("body bigger than listed");
    // a cancel before the download starts, and one that comes in after the download but before the swap, leave the old file alone
    let t = Transfer::default();
    t.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(install(&src, &good, &exe, &t, &|| false, WIN), Err("Cancelled.".to_string()));
    untouched("cancelled at the start");
    let t = Transfer::default();
    let calls = Cell::new(0);
    let cancel_at_the_end = || {
        calls.set(calls.get() + 1);
        if calls.get() == 2 {
            t.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        false
    };
    assert_eq!(install(&src, &good, &exe, &t, &cancel_at_the_end, WIN), Err("Cancelled.".to_string()));
    untouched("cancelled before the swap");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn an_oversized_manifest_is_refused() {
    let d = scratch("bigmanifest");
    let big = vec![b' '; MANIFEST_MAX as usize + 1];
    releases::write_test_release(&d, "v0.2.0", "", &[(MANIFEST_ASSET, &big), (file_name(WIN), &fake_lib(WIN, 2))], &|_, h| Some(format!("sha256:{h}")));
    assert_eq!(fetch(&ReleaseSource::Folder(d.clone()), &d.join("tmp")).unwrap_err(), ReleaseError::TooLarge);
    assert!(!d.join("tmp").join(MANIFEST_ASSET).exists());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn only_download_addresses_of_the_pinned_repo_are_followed() {
    let gh = ReleaseSource::github(REPO);
    let asset = |u: &str| Asset { name: "x".into(), size: 1, browser_download_url: u.into(), digest: None };
    let good = "https://github.com/AsobiCodes/omniloader/releases/download/v0.2.0/omniloader-windows-x86_64.dll";
    assert_eq!(url_ok(&gh, &asset(good)), Ok(()));
    assert_eq!(url_ok(&gh, &asset(&good.replace("AsobiCodes", "asobicodes"))), Ok(()), "GitHub does not care about letter case");
    for bad in [
        "http://github.com/AsobiCodes/omniloader/releases/download/v0.2.0/x",
        "https://evil.example/AsobiCodes/omniloader/releases/download/v0.2.0/x",
        "https://github.com.evil.example/AsobiCodes/omniloader/releases/download/v0.2.0/x",
        "https://github.com@evil.example/AsobiCodes/omniloader/releases/download/v0.2.0/x",
        "https://github.com/AsobiCodes/other/releases/download/v0.2.0/x",
        "https://github.com/AsobiCodes/omniloader2/releases/download/v0.2.0/x",
        "https://github.com/AsobiCodes/omniloader/releases/download/",
        "https://github.com/AsobiCodes/omniloader/archive/refs/heads/main.zip",
        "ftp://github.com/AsobiCodes/omniloader/releases/download/v0.2.0/x",
        "omniloader.dll",
        "",
    ] {
        let e = url_ok(&gh, &asset(bad)).unwrap_err();
        assert!(e.contains("github.com/AsobiCodes/omniloader"), "{bad}: {e}");
    }
    // a folder source (tests, RAC_LOADER_RELEASES) has file names, not addresses
    assert_eq!(url_ok(&ReleaseSource::Folder(PathBuf::from("x")), &asset("omniloader.dll")), Ok(()));
    // install refuses a foreign address before any network is touched
    let d = scratch("url");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let g = d.join("game");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&old, None)));
    let mut o = offer_of(&d.join("rel"));
    o.asset.browser_download_url = "http://example.invalid/omniloader.dll".into();
    let e = install(&gh, &o, &exe, &Transfer::default(), &|| false, WIN).unwrap_err();
    assert!(e.contains("outside github.com/AsobiCodes/omniloader"), "{e}");
    assert_eq!(std::fs::read(loader::lib_path(&exe)).unwrap(), old);
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------------------------------------- an offer is checked again before it is installed

#[test]
fn an_offer_is_judged_again_against_the_game_and_the_file_before_the_install() {
    let d = scratch("still");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d, "0.2.0", "1.7", "1.5", &new, None, None);
    let o = offer_of(&d);
    assert_eq!(o.min_host, Abi { major: 1, minor: 5 });
    let inst = unknown_installed(&old);
    assert_eq!(still_offered(&o, host(6), &inst), Ok(()));
    assert_eq!(still_offered(&o, host(5), &inst), Ok(()));
    // the offer was made for a game that served 1.6; this one serves 1.3, or another major
    assert!(still_offered(&o, host(3), &inst).unwrap_err().contains("1.5 or newer"));
    assert!(still_offered(&o, Abi { major: 2, minor: 0 }, &inst).unwrap_err().contains("older game"));
    assert!(still_offered(&o, Abi { major: 0, minor: 9 }, &inst).unwrap_err().contains("newer game"));
    // the installed loader became newer, or became exactly this file
    let mut newer = inst.clone();
    newer.version = Some("0.3.0".into());
    assert!(still_offered(&o, host(6), &newer).unwrap_err().contains("already up to date"));
    assert!(still_offered(&o, host(6), &unknown_installed(&new)).unwrap_err().contains("already up to date"));
    // nothing installed at all is fine
    assert_eq!(still_offered(&o, host(6), &Installed::default()), Ok(()));
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------------------------------------- an unknown installed loader counts as 0.1.0

fn sidecar(version: &str, sha: Option<&str>) -> String {
    match sha {
        Some(h) => format!(r#"{{"version":"{version}","abi":"1.6","sha256":"{h}"}}"#),
        None => format!(r#"{{"version":"{version}","abi":"1.6"}}"#),
    }
}

#[test]
fn a_loader_nobody_vouches_for_is_unknown_and_taken_for_the_first_version() {
    let d = scratch("unknown");
    let lib = fake_lib(WIN, 1);
    let other = hex(b"another file");
    let cases: Vec<(&str, Option<String>)> = vec![
        ("no sidecar", None),
        ("stale sidecar claiming a higher version", Some(sidecar("9.0.0", Some(&other)))),
        ("sidecar without a sha256", Some(sidecar("9.0.0", None))),
        ("version that is no version", Some(sidecar("latest", Some(&hex(&lib))))),
        ("two-part version", Some(sidecar("1.2", Some(&hex(&lib))))),
        ("leading zero", Some(sidecar("01.2.3", Some(&hex(&lib))))),
        ("empty version", Some(sidecar("", Some(&hex(&lib))))),
        ("unreadable sidecar", Some("{ not json".to_string())),
    ];
    for (i, (what, side)) in cases.iter().enumerate() {
        let dir = d.join(format!("g{i}"));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = game(&dir, Some((&lib, side.as_deref())));
        let inst = installed(&exe);
        assert_eq!((inst.present, inst.version.clone(), inst.abi), (true, None, None), "{what}");
        assert_eq!(inst.sha256, Some(hex(&lib)), "{what}");
        // the first public loader is not offered over it, a newer one is
        release(&d.join("r010"), "0.1.0", "1.6", "1.0", &fake_lib(WIN, 3), None, None);
        assert!(matches!(verdict_for(&d.join("r010"), host(6), &inst), Verdict::UpToDate(_)), "{what}: 0.1.0 offered over an unknown file");
        release(&d.join("r020"), "0.2.0", "1.6", "1.0", &fake_lib(WIN, 4), None, None);
        assert!(matches!(verdict_for(&d.join("r020"), host(6), &inst), Verdict::Offer(_)), "{what}: 0.2.0 not offered");
        // the very same bytes in the release are "up to date" whatever the sidecar says
        release(&d.join("rsame"), "0.2.0", "1.6", "1.0", &lib, None, None);
        assert!(matches!(verdict_for(&d.join("rsame"), host(6), &inst), Verdict::UpToDate(_)), "{what}: same bytes");
    }
    // a sidecar that names this file (any letter case) with a real version is believed
    let dir = d.join("good");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = game(&dir, Some((&lib, Some(&sidecar("0.3.0", Some(&hex(&lib).to_uppercase()))))));
    let inst = installed(&exe);
    assert_eq!((inst.version.as_deref(), inst.abi), (Some("0.3.0"), Some(Abi { major: 1, minor: 6 })));
    assert!(matches!(verdict_for(&d.join("r020"), host(6), &inst), Verdict::UpToDate(_)));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn pre_releases_sort_below_their_release_both_ways() {
    let d = scratch("pre");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    let at = |have: &str| Installed { present: true, version: Some(have.to_string()), abi: Some(Abi { major: 1, minor: 6 }), sha256: Some(hex(&old)) };
    // (installed, release, offered?)
    for (have, rel, offered) in [
        ("0.2.0", "0.2.0-rc.1", false),
        ("0.2.0-rc.1", "0.2.0", true),
        ("0.2.0-rc.2", "0.2.0-rc.10", true),
        ("0.2.0-rc.10", "0.2.0-rc.2", false),
        ("0.2.0-rc.1", "0.2.0-rc.1", false),
        ("0.1.9", "0.2.0-rc.1", true),
        ("0.2.0-beta", "0.2.0-rc.1", true),
    ] {
        release(&d, rel, "1.6", "1.0", &new, None, None);
        let v = verdict_for(&d, host(6), &at(have));
        assert_eq!(matches!(v, Verdict::Offer(_)), offered, "installed {have}, release {rel}: {v:?}");
    }
    // a release whose own version cannot be read is never offered (the manifest is refused)
    for bad in ["nightly", "1.2", "01.2.3", "1.2.3.", "1.0.0-rc..1", "1.2.3.4", "1.0.0-"] {
        release(&d, bad, "1.6", "1.0", &new, None, None);
        assert!(fetch(&ReleaseSource::Folder(d.clone()), &d.join("tmp")).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn the_old_sidecar_stays_with_its_file_and_a_stale_one_is_dropped() {
    let d = scratch("carry");
    let (old, new) = (fake_lib(WIN, 1), fake_lib(WIN, 2));
    release(&d.join("rel"), "0.2.0", "1.7", "1.0", &new, None, None);
    let src = ReleaseSource::Folder(d.join("rel"));
    let leftover = r#"{"version":"0.0.9","abi":"1.6","sha256":"left over from an earlier swap"}"#;
    // an old file with no sidecar, an old leftover .json.prev: the leftover goes, nothing sits beside the new .prev
    // an old file with a stale sidecar: the same
    for (i, side) in [None, Some(sidecar("9.0.0", Some(&hex(b"some other file"))))].iter().enumerate() {
        let g = d.join(format!("game{i}"));
        std::fs::create_dir_all(&g).unwrap();
        let exe = game(&g, Some((&old, side.as_deref())));
        let lib = loader::lib_path(&exe);
        std::fs::write(with_suffix(&lib, ".json.prev"), leftover).unwrap();
        install(&src, &offer_of(&d.join("rel")), &exe, &Transfer::default(), &|| false, WIN).unwrap();
        assert_eq!(std::fs::read(with_suffix(&lib, ".prev")).unwrap(), old, "case {i}");
        assert!(!with_suffix(&lib, ".json.prev").exists(), "case {i}: a .prev sits beside a sidecar that is not its own");
        assert_eq!(installed(&exe).version.as_deref(), Some("0.2.0"), "case {i}");
        // going back: the old file comes back without any sidecar (it is unknown again), the new file's sidecar goes with the new file
        roll_back(&exe, &|| false).unwrap();
        assert_eq!(std::fs::read(&lib).unwrap(), old, "case {i}");
        assert!(!with_suffix(&lib, ".json").exists() && with_suffix(&lib, ".json.prev").is_file(), "case {i}");
        assert_eq!(installed(&exe).version, None, "case {i}");
        roll_back(&exe, &|| false).unwrap();
        assert_eq!(installed(&exe).version.as_deref(), Some("0.2.0"), "case {i}");
    }
    // the game's own loader with a sidecar that does not name it: put back, but without that sidecar
    let root = d.join("root");
    std::fs::create_dir_all(root.join("bundled")).unwrap();
    std::fs::write(bundled_lib(&root, WIN), &old).unwrap();
    std::fs::write(with_suffix(&bundled_lib(&root, WIN), ".json"), sidecar("0.1.0", None)).unwrap();
    let g = d.join("game_b");
    std::fs::create_dir_all(&g).unwrap();
    let exe = game(&g, Some((&new, Some(&sidecar("0.2.0", Some(&hex(&new)))))));
    assert_eq!(restore_bundled(&root, &exe, &|| false, WIN), Ok(true));
    let lib = loader::lib_path(&exe);
    assert_eq!(std::fs::read(&lib).unwrap(), old);
    assert!(!with_suffix(&lib, ".json").exists(), "a sidecar without a sha256 was passed on");
    assert_eq!(installed(&exe).version, None);
    assert!(with_suffix(&lib, ".json.prev").is_file(), "the replaced file's own sidecar is kept with it");
    let _ = std::fs::remove_dir_all(&d);
}
