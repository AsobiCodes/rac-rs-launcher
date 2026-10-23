# RAC-RS Launcher

A small desktop app (Rust, egui) that installs, updates, configures and starts the PC port of Ratchet & Clank from
[AsobiCodes/rac-rs](https://github.com/AsobiCodes/rac-rs). Bring your own disc: the game is built on your PC from your own disc image; nothing of the game is shipped here.

## Download
Get `rac-rs-launcher-windows-x64.exe` from the [Releases](../../releases) page (the release shows its SHA-256), rename it to `RAC-RS Launcher.exe` if you like and double-click it.
The launcher checks for a new launcher and a new game version and asks before it installs either. It updates itself only from this repo.

## Install a game
1. Library, "New install": pick a folder and a game version (default: the newest; older ones from 0.2.1 are installed side by side and stay on that version).
2. The launcher downloads the release from GitHub (checksum checked), then asks for your disc image (.iso, PAL or USA v1.00) and builds the game (about 15 minutes, once).
3. Play.

## Profiles, mods, texture packs
Each install has profiles (the game's options and the active mods). Mods and texture packs are "bring your own pack": add a folder or .zip, or import from your PCSX2 texture folder.
Saves: "Open saves folder" opens the install's saves folder (copy `card.bin` between installs, game closed).

## Reporting a problem
"Open log folder" (Game page, sidebar, or Settings > Advanced) opens the folder with `log.txt` selected: attach it to an issue on [AsobiCodes/rac-rs](https://github.com/AsobiCodes/rac-rs/issues). If you already restarted the game, attach `log.prev.txt` too.

## Build from source
`cd launcher; cargo build --release` (Rust stable). `build_release.ps1` makes `dist\rac-rs-launcher-windows-x64.exe` and its SHA-256. `check_linux.ps1` checks the Linux build.

## License
GPL-3.0-or-later, see LICENSE. The libraries built into the exe and their licences are listed in THIRD_PARTY.md.
