//! Linux: "Add to Steam" in the window (see `steam.rs`): the tick on the install page, and the Add / Remove button in Settings > Advanced.
//! Windows never shows any of it (`SteamUi::shown` is false there; tests switch it on).
use super::state::App;
use super::theme;
use crate::steam::{self, Kind, Mode, Outcome, State};
use egui::Ui;
use std::path::Path;

/// What the window knows about Steam.
pub struct SteamUi {
    /// Show the tick and the button (Linux).
    pub shown: bool,
    /// The install page's tick "Add the game to Steam" (off until the player ticks it, as in the Windows setup window).
    pub wanted: bool,
    /// Steam found / the game in it, as last read (None = not read yet, or the tool could not say).
    pub state: Option<State>,
    /// The state was read for this folder.
    pub read_for: Option<std::path::PathBuf>,
    /// The call that does the work (tests replace it).
    pub run: fn(&Path, Mode) -> Outcome,
    /// The call that reads the state (tests replace it).
    pub read: fn(&Path) -> Option<State>,
}

impl Default for SteamUi {
    fn default() -> Self {
        SteamUi { shown: cfg!(not(windows)), wanted: false, state: None, read_for: None, run: steam::run, read: steam::state }
    }
}

/// The sentence the Settings button and the install note show for a result.
pub fn result_note(o: &Outcome) -> String {
    match o.kind {
        Kind::Running => format!("{} Nothing was changed.", o.text),
        _ => o.text.clone(),
    }
}

impl App {
    /// Reads whether Steam and the game's entry are there (once per folder; again after every change).
    pub fn steam_refresh(&mut self, force: bool) {
        if !self.steam.shown || !self.has_install() {
            return;
        }
        if !force && self.steam.read_for.as_deref() == Some(self.root.as_path()) {
            return;
        }
        self.steam.state = (self.steam.read)(&self.root);
        self.steam.read_for = Some(self.root.clone());
    }

    /// The Settings button: add the game to Steam, or take it out when it is in.
    pub fn steam_toggle(&mut self) {
        self.steam_refresh(false);
        let mode = if self.steam.state.map(|s| s.has).unwrap_or(false) { Mode::Remove } else { Mode::Add };
        let o = (self.steam.run)(&self.root, mode);
        self.toast(result_note(&o), o.kind != Kind::Done);
        self.steam_refresh(true);
    }

    /// After a first install with the tick set: try to add the game, and say what happened on the done page.
    pub fn steam_after_install(&mut self) -> Option<String> {
        if !self.steam.shown || !self.steam.wanted {
            return None;
        }
        let o = (self.steam.run)(&self.root, Mode::Add);
        self.steam_refresh(true);
        Some(match o.kind {
            Kind::Done => o.text,
            Kind::Running => format!("The game is not in Steam yet: {} Close Steam, then use \"Add to Steam\" in Settings (Advanced).", o.text),
            _ => format!("The game could not be added to Steam: {}", o.text),
        })
    }

    /// The tick on the install page (Linux only).
    pub fn steam_tick(&mut self, ui: &mut Ui) {
        if self.steam.shown {
            ui.checkbox(&mut self.steam.wanted, "Add the game to Steam (for the Steam Deck; Steam must be closed)");
        }
    }

    /// The card in Settings > Advanced (Linux only).
    pub fn steam_card(&mut self, ui: &mut Ui) {
        if !self.steam.shown || !self.has_install() || self.is_backup() {
            return;
        }
        self.steam_refresh(false);
        ui.separator();
        ui.label(theme::bold("Steam (Steam Deck)", 15.5));
        ui.label(theme::muted("Adds the game to your Steam library as a non-Steam game, so it also starts from Gaming Mode with the controller. Steam must be closed first (on the Steam Deck: Desktop Mode, then Steam menu > Exit)."));
        match self.steam.state {
            Some(State { found: false, .. }) => {
                ui.label(theme::muted("Steam was not found for this user. Start Steam once, sign in and close it, then come back."));
            }
            Some(State { has: true, .. }) => {
                ui.label(theme::muted("The game is in your Steam library."));
            }
            _ => {}
        }
        let label = steam::button_label(self.steam.state);
        if theme::button_enabled(ui, label, !self.game_running()) {
            self.steam_toggle();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(text: &str) -> Outcome {
        Outcome { kind: Kind::Done, text: text.to_string() }
    }

    #[test]
    fn a_result_is_the_tools_own_sentence() {
        assert_eq!(result_note(&done("Added it.")), "Added it.");
        let r = Outcome { kind: Kind::Running, text: "Steam is running. Close Steam first.".to_string() };
        assert_eq!(result_note(&r), "Steam is running. Close Steam first. Nothing was changed.");
        let n = Outcome { kind: Kind::NoSteam, text: "Steam was not found on this PC.".to_string() };
        assert_eq!(result_note(&n), "Steam was not found on this PC.");
    }

    #[test]
    fn the_windows_launcher_shows_nothing() {
        let s = SteamUi::default();
        assert_eq!(s.shown, cfg!(not(windows)));
        assert!(!s.wanted, "the tick starts off");
    }
}
