# Changelog

## v0.1.0
First release.
- Install the PC port of Ratchet & Clank from your own disc: the launcher downloads the game release from AsobiCodes/rac-rs (SHA-256 checked), asks for your disc image (.iso, PAL or USA v1.00) and builds the game once (about 15 minutes).
- Library of installs: several game versions side by side (from 0.2.1), each keeps its version. Profiles per install, each with its own settings and mods.
- Settings page: the game's options, plus "Graphics quality" presets Low / Medium / High / Custom. High is the game's own default picture; Low and Medium turn the costly options down (draw distance, frame smoothing, HUD sharpness, shadows, water, particles and so on). A preset never touches camera, controls, language, window mode or the graphics system; change any option afterwards and the selector shows Custom.
- Mods page ("bring your own pack"): add a texture pack as a folder or .zip, by drag and drop, or import it from your PCSX2 texture folder. Big packs (6 GB and more) are read and copied in the background with a progress bar and Cancel. Per-pack switches and priority order, plus two master switches, "Load texture packs" and "Load mods", that turn a whole kind off without losing the per-pack choices.
- A pack made for the other disc region is marked: most world textures still match; menus, HUD and text may stay original.
- Options and features an older game version does not have are greyed out (texture packs and the graphics-system choice need game version 0.3.5).
- "Open saves folder" and "Open log folder" (with "Report a problem"): the log folder opens with `log.txt` selected, the file to attach to a bug report.
- Updates: the launcher checks for a new launcher and a new game version and asks before it installs either.
- Windows first; Linux groundwork only (it compiles, there is no Linux install yet).
