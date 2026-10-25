//! Overview screen: grouping, folding, clock, and clicks. Headless (iced_test Simulator, tiny-skia); no window.

use fb_ui::Mode;
use flockbook::overview::{self, age_label, columns_for, groups, Clock, GroupBy, OverviewMsg};
use flockbook::snapshot::{distinct_colors, render_app, write_png};
use flockbook::state::Screen;
use flockbook::{App, Message};
use iced::{Settings, Size};
use iced_test::Simulator;

fn app(mode: Mode) -> App {
    App::demo(mode, Screen::Overview)
}

fn sim(app: &App, w: f32, h: f32) -> Simulator<'_, Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    Simulator::with_size(settings, Size::new(w, h), app.view())
}

fn names<'a>(g: &'a [overview::Entry<'a>]) -> Vec<String> {
    g.iter().flat_map(|e| e.tab.panes.iter().filter_map(|p| p.agent.as_ref().map(|a| a.name.clone()))).collect()
}

#[test]
fn group_sizes_match_the_demo_data() {
    let a = app(Mode::Light);
    let g = groups(&a.state.workspaces);
    assert_eq!((g.blocked.len(), g.working.len(), g.done.len(), g.idle.len()), (2, 6, 5, 3));
}

#[test]
fn every_tab_is_a_card_under_its_state_heading() {
    // Swift `OverviewView`: Needs you, Working, then Done and resting, all as cards (`Screens.swift:186-191`).
    let a = app(Mode::Light);
    let g = groups(&a.state.workspaces);
    let mut s = sim(&a, 1280.0, 1400.0);
    for n in names(&g.blocked).iter().chain(names(&g.working).iter()).chain(names(&g.done).iter()).chain(names(&g.idle).iter()) {
        assert!(s.find(n.as_str()).is_ok(), "card {n} missing");
    }
    for h in ["Needs you", "Working", "Done and resting"] {
        assert!(s.find(h).is_ok(), "heading {h} missing");
    }
    assert!(s.find("Good afternoon").is_ok());
    assert!(s.find("16 tabs \u{b7} 6 working \u{b7} 2 need you").is_ok());
}

#[test]
fn a_card_shows_its_age_but_no_preview_or_state_line() {
    let a = app(Mode::Light);
    let mut s = sim(&a, 1280.0, 1400.0);
    assert!(s.find("24m").is_ok(), "the age stays on the card");
    for gone in ["Review", "Asks to run: ./scripts/deploy.sh --env staging"] {
        assert!(s.find(gone).is_err(), "{gone} is not on a card");
    }
}

#[test]
fn clicking_a_card_opens_its_conversation() {
    let mut a = app(Mode::Light);
    assert_eq!(a.state.screen, Screen::Overview);
    let msgs: Vec<Message> = {
        let mut s = sim(&a, 1280.0, 852.0);
        s.click("harbor-deploy-ops").expect("card");
        s.into_messages().collect()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Overview(OverviewMsg::Open { .. }))), "no Open message: {msgs:?}");
    for m in msgs {
        let _ = a.update(m);
    }
    assert_eq!(a.state.screen, Screen::Conversation);
    assert_eq!(a.state.selected_tab().map(|t| t.label.as_str()), Some("deploy"));
    assert_eq!(a.state.selected_workspace().map(|w| w.label.as_str()), Some("Harbor"));
}

#[test]
fn workspace_grouping_lists_every_tab() {
    let mut a = app(Mode::Light);
    let _ = a.update(Message::Overview(OverviewMsg::SetGroup(GroupBy::Workspace)));
    let g = groups(&a.state.workspaces);
    let mut s = sim(&a, 1280.0, 1400.0);
    for n in names(&g.blocked).iter().chain(&names(&g.working)).chain(&names(&g.done)).chain(&names(&g.idle)) {
        assert!(s.find(n.as_str()).is_ok(), "{n} missing in workspace grouping");
    }
    assert!(s.find("1 need you \u{b7} 2 resting").is_ok());
}

#[test]
fn greeting_follows_the_injected_clock() {
    for (hour, word) in [(6, "Good morning"), (13, "Good afternoon"), (20, "Good evening")] {
        let mut a = app(Mode::Light);
        a.state.overview.clock = Some(Clock { hour, now: 0.0 });
        let mut s = sim(&a, 1280.0, 852.0);
        assert!(s.find(word).is_ok(), "{word}");
    }
}

#[test]
fn ages_and_columns() {
    assert_eq!(age_label(100.0, 90.0), "now");
    assert_eq!(age_label(1000.0, 700.0), "5m");
    assert_eq!(age_label(10_000.0, 0.0), "2h");
    assert_eq!(age_label(200_000.0, 0.0), "2d");
    assert_eq!(columns_for(500.0), 1);
    assert_eq!(columns_for(700.0), 2);
    assert_eq!(columns_for(1280.0), 3);
    assert_eq!(columns_for(4000.0), 3);
}

#[test]
fn preview_prefers_the_newest_readable_item() {
    let a = app(Mode::Light);
    let g = groups(&a.state.workspaces);
    let blocked = g.blocked[0].tab;
    let pane = blocked.panes.iter().find(|p| p.agent.is_some()).unwrap();
    let pv = overview::preview_of(&a.source.conversation(pane));
    assert!(pv.text.starts_with("Asks to run:"), "{}", pv.text);
    assert!(pv.timestamp.is_some());
}

#[test]
fn snapshots_light_and_dark() {
    for (mode, name) in [(Mode::Light, "overview-light"), (Mode::Dark, "overview-dark")] {
        let a = app(mode);
        let img = render_app(&a, (1280, 852));
        assert_eq!((img.width, img.height), (2560, 1704));
        assert!(distinct_colors(&img) > 40);
        if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
            write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{name}.png"))).unwrap();
        }
    }
}
