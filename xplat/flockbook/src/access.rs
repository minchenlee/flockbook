//! Whether the app may send anything to a pane (port of Swift `AppModel.writesEnabled`).
//!
//! Writes are on by default. They are off with `--read-only`, with `FLOCKBOOK_READ_ONLY=1`, or when the "Allow sending
//! input" setting is off (`settings.json`, read at launch by `main.rs`; "not set" = on, like Swift).
//! Read-only means no [`crate::composer::MessageSender`] and no [`crate::actions::PaneActions`] are installed: the composer
//! shows "Read-only: input is off" and the cards say "Answer in the terminal". The terminal sheet opens in Observe mode:
//! no key, paste, resize or wheel goes to the program.

use crate::App;
use std::collections::HashMap;

/// The Swift rule: `--read-only` (or the env flag) wins; else the setting, default on.
pub fn writes_enabled(args: &[String], env: &HashMap<String, String>, allow_input: Option<bool>) -> bool {
    if args.iter().any(|a| a == "--read-only") {
        return false;
    }
    if env.get("FLOCKBOOK_READ_ONLY").is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")) {
        return false;
    }
    allow_input.unwrap_or(true)
}

/// Whether this window sends input now: a composer sender is installed. The Settings sheet reads this, so it never
/// says "on" while the composer says "Read-only".
pub fn writes_live(app: &App) -> bool {
    app.state.composer.sender.is_some()
}

/// Takes every write path away from `app` (the composer's sender, the cards' pane actions) and redraws the cards.
pub fn make_read_only(app: &mut App) {
    if let Some(sender) = &app.state.composer.sender { sender.revoke(); }
    app.state.composer.sender = None;
    app.state.cards.actions = None;
    // Terminals open in Observe mode from now on; an open control sheet is released.
    app.state.terminal.writable = false;
    crate::terminal::close(app);
    app.refresh();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }
    fn env(k: &str, v: &str) -> HashMap<String, String> {
        HashMap::from([(k.to_string(), v.to_string())])
    }

    #[test]
    fn writes_are_on_by_default() {
        assert!(writes_enabled(&args(&[]), &HashMap::new(), None));
        assert!(writes_enabled(&args(&["--session", "x"]), &HashMap::new(), Some(true)));
    }

    #[test]
    fn the_flag_the_env_and_the_setting_each_turn_writes_off() {
        assert!(!writes_enabled(&args(&["--read-only"]), &HashMap::new(), None));
        assert!(!writes_enabled(&args(&["--read-only"]), &HashMap::new(), Some(true)), "the flag beats the setting");
        assert!(!writes_enabled(&args(&[]), &env("FLOCKBOOK_READ_ONLY", "1"), None));
        assert!(!writes_enabled(&args(&[]), &env("FLOCKBOOK_READ_ONLY", "TRUE"), Some(true)));
        assert!(writes_enabled(&args(&[]), &env("FLOCKBOOK_READ_ONLY", "0"), None), "0 does not turn it off");
        assert!(!writes_enabled(&args(&[]), &HashMap::new(), Some(false)));
    }

    #[test]
    fn read_only_app_has_no_sender_and_buttons_off() {
        let mut app = App::demo(fb_ui::Mode::Light, crate::state::Screen::Conversation);
        let app_ref = &mut app;
        let found = app_ref.state.workspaces.iter().find_map(|w| w.tabs.iter().find(|t| t.label == "deploy").map(|t| (w.id.clone(), t.id.clone()))).unwrap();
        app_ref.state.select_tab(&found.0, &found.1);
        app_ref.refresh();
        assert!(matches!(&app_ref.state.cards.card, Some(crate::cards::Card::Approval(a)) if a.can_act), "demo cards are live");
        make_read_only(app_ref);
        assert!(app_ref.state.composer.sender.is_none() && app_ref.state.cards.actions.is_none());
        assert!(matches!(&app_ref.state.cards.card, Some(crate::cards::Card::Approval(a)) if !a.can_act), "{:?}", app_ref.state.cards.card);
    }
}
