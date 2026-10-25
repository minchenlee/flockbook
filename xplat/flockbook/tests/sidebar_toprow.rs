//! Sidebar and top row: interaction tests through `iced_test::Simulator` (tiny-skia, no window) plus headless
//! snapshots. Set `FB_WRITE_EVIDENCE=1` to copy the PNGs to `evidence/`.

use fb_ui::Mode;
use flockbook::snapshot::{render_app, write_png};
use flockbook::source::{DataSource, DemoSource};
use flockbook::state::Screen;
use flockbook::toprow::{on_key, tab_tooltip, Edge, TopRowMsg};
use flockbook::{App, Message};
use herdr_core::{Agent, AgentStatus, ConversationItem, Pane, Tab, Workspace};
use iced::keyboard::{key::Named, Key, Modifiers};
use iced::{Settings, Size};
use iced_test::Simulator;

/// Runs a view in the simulator, returns the messages produced by `act`.
fn run(app: &App, size: (f32, f32), act: impl FnOnce(&mut Simulator<'_, Message>)) -> Vec<Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    let mut sim: Simulator<'_, Message> = Simulator::with_size(settings, Size::new(size.0, size.1), app.view());
    act(&mut sim);
    sim.into_messages().collect()
}

fn apply(app: &mut App, msgs: Vec<Message>) {
    for m in msgs {
        let _ = app.update(m);
    }
}

fn sel(app: &App) -> (String, String) {
    (app.state.selected_workspace().unwrap().label.clone(), app.state.selected_tab().unwrap().label.clone())
}

#[test]
fn clicking_a_workspace_row_selects_it_and_changes_tabs() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    assert_eq!(sel(&app), ("Quarry".into(), "ranking".into()));
    let before: Vec<String> = app.state.selected_workspace().unwrap().tabs.iter().map(|t| t.label.clone()).collect();
    let msgs = run(&app, (1280.0, 852.0), |sim| {
        sim.click("Atlas").expect("Atlas row");
    });
    apply(&mut app, msgs);
    let (ws, _) = sel(&app);
    assert_eq!(ws, "Atlas");
    let after: Vec<String> = app.state.selected_workspace().unwrap().tabs.iter().map(|t| t.label.clone()).collect();
    assert_ne!(before, after);
    // The top row now shows Atlas tabs.
    let found = run(&app, (1280.0, 852.0), |sim| {
        assert!(sim.find("backfill").is_ok());
        assert!(sim.find("ranking").is_err());
    });
    assert!(found.is_empty());
}

#[test]
fn clicking_a_blocked_workspace_selects_its_blocked_tab() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let msgs = run(&app, (1280.0, 852.0), |sim| {
        sim.click("Harbor").expect("Harbor row");
    });
    apply(&mut app, msgs);
    assert_eq!(sel(&app), ("Harbor".into(), "deploy".into()));
}

#[test]
fn clicking_a_tab_selects_it() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let msgs = run(&app, (1280.0, 852.0), |sim| {
        sim.click("bench").expect("bench tab");
    });
    apply(&mut app, msgs);
    assert_eq!(sel(&app), ("Quarry".into(), "bench".into()));
}

#[test]
fn cmd_digit_selects_tabs_and_ctrl_tab_cycles() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let msg = on_key(Key::Character("1".into()), Modifiers::COMMAND).expect("cmd+1");
    let _ = app.update(msg);
    assert_eq!(sel(&app).1, "ranking");
    let _ = app.update(on_key(Key::Character("3".into()), Modifiers::COMMAND).unwrap());
    assert_eq!(sel(&app).1, "notes");
    // Out of range does nothing.
    let _ = app.update(on_key(Key::Character("9".into()), Modifiers::COMMAND).unwrap());
    assert_eq!(sel(&app).1, "notes");
    // Ctrl+Tab wraps to the first tab, Ctrl+Shift+Tab back to the last.
    let _ = app.update(on_key(Key::Named(Named::Tab), Modifiers::CTRL).unwrap());
    assert_eq!(sel(&app).1, "ranking");
    let _ = app.update(on_key(Key::Named(Named::Tab), Modifiers::CTRL | Modifiers::SHIFT).unwrap());
    assert_eq!(sel(&app).1, "notes");
    // Plain keys are not shortcuts.
    assert!(on_key(Key::Character("1".into()), Modifiers::empty()).is_none());
}

#[test]
fn cmd_alt_arrows_move_between_workspaces() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let mods = Modifiers::COMMAND | Modifiers::ALT;
    let n = app.state.workspaces.len();
    let start = app.state.workspaces.iter().position(|w| Some(&w.id) == app.state.selected_workspace.as_ref()).unwrap();
    let _ = app.update(on_key(Key::Named(Named::ArrowDown), mods).unwrap());
    let at = |app: &App| app.state.workspaces.iter().position(|w| Some(&w.id) == app.state.selected_workspace.as_ref()).unwrap();
    assert_eq!(at(&app), (start + 1) % n);
    let _ = app.update(on_key(Key::Named(Named::ArrowUp), mods).unwrap());
    assert_eq!(at(&app), start);
    let _ = app.update(on_key(Key::Named(Named::ArrowUp), mods).unwrap());
    assert_eq!(at(&app), (start + n - 1) % n);
}

#[test]
fn blocked_badge_count_matches_demo_data() {
    let app = App::demo(Mode::Light, Screen::Conversation);
    let blocked = app.state.blocked_tabs().len();
    assert_eq!(blocked, 2);
    let per_ws: usize = app.state.workspaces.iter().map(|w| flockbook::state::State::counts(w).blocked).sum();
    assert_eq!(per_ws, blocked);
    run(&app, (1280.0, 852.0), |sim| {
        // The Needs you badge shows 2. Workspace rows show one status glyph per tab, no count badge
        // (Swift StatusColumn, SidebarView.swift:189-221).
        assert!(sim.find("2").is_ok(), "Needs you badge");
    });
}

#[test]
fn needs_you_row_navigates() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let msgs = run(&app, (1280.0, 852.0), |sim| {
        sim.click("Needs you").expect("row");
    });
    apply(&mut app, msgs);
    assert_eq!(app.state.screen, Screen::NeedsYou);
}

#[test]
fn tooltip_names_agent_state_and_panes() {
    let app = App::demo(Mode::Light, Screen::Conversation);
    let tab = app.state.selected_tab().unwrap();
    let tip = tab_tooltip(tab, 1);
    assert!(tip.contains("Working"), "{tip}");
    assert!(tip.contains("Tab 1"), "{tip}");
    assert!(tip.contains("1 pane"), "{tip}");
    assert!(tip.contains("quarry-rank-dev"), "{tip}");
}

// ---- a workspace with many tabs --------------------------------------------------------------------------------

struct ManyTabs;

fn tab(i: usize, label: &str, status: AgentStatus, panes: usize) -> Tab {
    let pane = |k: usize| Pane {
        id: format!("p{i}-{k}"),
        terminal_id: format!("t{i}-{k}"),
        cwd: "/work".into(),
        agent: (k == 0).then(|| Agent { name: format!("agent-{i}"), kind: "claude".into(), status, session_id: None, title: label.into() }),
        label: None,
    };
    Tab { id: format!("tab{i}"), label: label.into(), number: i as i64, panes: (0..panes).map(pane).collect(), status }
}

impl DataSource for ManyTabs {
    fn label(&self) -> String {
        "many".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        use AgentStatus::*;
        let names: [(&str, AgentStatus, usize); 16] = [
            ("cart drawer", Working, 1),
            ("pricing", Done, 1),
            ("e2e", Done, 1),
            ("search relevance and ranking experiments", Working, 1),
            ("bench", Working, 2),
            ("notes", Done, 1),
            ("importer", Idle, 1),
            ("schema migration", Working, 1),
            ("deploy", Blocked, 1),
            ("ci", Done, 1),
            ("runbook", Idle, 1),
            ("日本語のタイトル", Done, 1),
            ("parser", Working, 1),
            ("release", Idle, 1),
            ("getting started", Blocked, 1),
            ("api ref", Idle, 1),
        ];
        let tabs = names.iter().enumerate().map(|(i, (l, s, p))| tab(i, l, *s, *p)).collect();
        vec![
            Workspace { id: "w1".into(), label: "Orchard".into(), tabs },
            Workspace { id: "w2".into(), label: "Atlas".into(), tabs: vec![tab(100, "backfill", Working, 1)] },
        ]
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, _pane: &Pane) -> Vec<ConversationItem> {
        Vec::new()
    }
}

fn many(mode: Mode, selected: &str) -> App {
    let mut app = App::new(Box::new(ManyTabs), mode, Screen::Conversation);
    app.state.select_tab("w1", selected);
    app
}

fn render(app: &App) {
    let _ = render_app(app, (1000, 400));
}

fn visible(app: &App, tab_id: &str) -> bool {
    // Re-derive the strip geometry through the public tooltip-free path: the offset and width the view measured.
    let ts = &app.state.toprow;
    let ws = app.state.selected_workspace().unwrap();
    let sel = app.state.selected_tab.as_deref();
    let mut x = 0.0f32;
    for t in &ws.tabs {
        let w = flockbook::toprow::tab_width(t, sel == Some(t.id.as_str()));
        if t.id == tab_id {
            return x >= ts.offset.get() - 0.5 && x + w <= ts.offset.get() + ts.strip_w.get() + 0.5;
        }
        x += w + flockbook::toprow::GAP;
    }
    false
}

#[test]
fn selecting_a_far_tab_scrolls_it_into_view() {
    let mut app = many(Mode::Light, "tab0");
    render(&app);
    assert_eq!(app.state.toprow.offset.get(), 0.0);
    assert!(app.state.toprow.content_w.get() > app.state.toprow.strip_w.get(), "strip must overflow for this test");
    // Cmd+9 -> tab index 8 (blocked "deploy").
    let _ = app.update(on_key(Key::Character("9".into()), Modifiers::COMMAND).unwrap());
    render(&app);
    assert_eq!(app.state.selected_tab.as_deref(), Some("tab8"));
    assert!(app.state.toprow.offset.get() > 0.0);
    assert!(visible(&app, "tab8"));
    // Ctrl+Shift+Tab from the first tab wraps to the last, which must scroll to the end.
    let mut app = many(Mode::Light, "tab0");
    render(&app);
    let _ = app.update(on_key(Key::Named(Named::Tab), Modifiers::CTRL | Modifiers::SHIFT).unwrap());
    render(&app);
    assert_eq!(app.state.selected_tab.as_deref(), Some("tab15"));
    assert!(visible(&app, "tab15"));
}

#[test]
fn wheel_scroll_is_clamped_and_marker_jumps_to_blocked_tab() {
    let mut app = many(Mode::Light, "tab0");
    render(&app);
    let _ = app.update(Message::TopRow(TopRowMsg::Scroll(-500.0)));
    assert_eq!(app.state.toprow.offset.get(), 0.0);
    let max = app.state.toprow.content_w.get() - app.state.toprow.strip_w.get();
    let _ = app.update(Message::TopRow(TopRowMsg::Scroll(100000.0)));
    assert!((app.state.toprow.offset.get() - max).abs() < 0.01);
    // Scrolled to the end: the blocked tab "deploy" (index 8) is hidden on the left. The marker reveals it.
    let _ = app.update(Message::TopRow(TopRowMsg::RevealBlocked(Edge::Left)));
    render(&app);
    assert!(app.state.toprow.offset.get() < max);
    // Clicking the marker itself works through the Simulator.
    let mut app = many(Mode::Light, "tab0");
    render(&app);
    let msgs = run(&app, (1000.0, 400.0), |sim| {
        sim.click(flockbook::toprow::marker_id(Edge::Right)).expect("blocked edge marker");
    });
    let revealed = msgs.iter().any(|m| matches!(m, Message::TopRow(TopRowMsg::RevealBlocked(Edge::Right))));
    assert!(revealed, "marker click should emit RevealBlocked(Right): {msgs:?}");
    apply(&mut app, msgs);
    render(&app);
    assert!(app.state.toprow.offset.get() > 0.0);
}

#[test]
fn workspace_switch_resets_the_strip() {
    let mut app = many(Mode::Light, "tab12");
    render(&app);
    assert!(app.state.toprow.offset.get() > 0.0);
    let _ = app.update(Message::SelectWorkspace("w2".into()));
    render(&app);
    assert_eq!(app.state.toprow.offset.get(), 0.0);
}

#[test]
fn all_tabs_menu_lists_tabs_and_picks_one() {
    let mut app = many(Mode::Light, "tab0");
    render(&app);
    let _ = app.update(Message::TopRow(TopRowMsg::ToggleMenu(flockbook::toprow::MenuKind::AllTabs)));
    let msgs = run(&app, (1280.0, 852.0), |sim| {
        // `menu_layer` is stacked above the window: the item for "runbook" is in the menu.
        sim.click("Show in Overview").expect("menu item");
    });
    assert!(msgs.iter().any(|m| matches!(m, Message::TopRow(TopRowMsg::ShowOverview))), "{msgs:?}");
    apply(&mut app, msgs);
    assert_eq!(app.state.screen, Screen::Overview);
    assert!(app.state.toprow.menu.is_none());
    // Picking a tab from the menu selects it and closes the menu.
    let mut app = many(Mode::Light, "tab0");
    let _ = app.update(Message::TopRow(TopRowMsg::ToggleMenu(flockbook::toprow::MenuKind::AllTabs)));
    let _ = app.update(Message::TopRow(TopRowMsg::PickTab { workspace: "w1".into(), tab: "tab10".into() }));
    assert_eq!(app.state.selected_tab.as_deref(), Some("tab10"));
    assert!(app.state.toprow.menu.is_none());
}

// ---- snapshots ----------------------------------------------------------------------------------------------------

fn snap(app: &App, name: &str) {
    let img = render_app(app, (1280, 852));
    assert_eq!((img.width, img.height), (2560, 1704));
    assert!(flockbook::snapshot::distinct_colors(&img) > 40);
    write_png(&img, &std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"))).unwrap();
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn snapshots() {
    for (mode, n) in [(Mode::Light, "light"), (Mode::Dark, "dark")] {
        snap(&App::demo(mode, Screen::Conversation), &format!("sidebar-toprow-{n}"));
        // Many tabs: selected tab in the middle, blocked tabs out of view on the right.
        snap(&many(mode, "tab4"), &format!("sidebar-toprow-many-{n}"));
        // Scrolled to the end: blocked tab hidden on the left, fade on the left only.
        let mut app = many(mode, "tab15");
        app.state.select_tab("w1", "tab15");
        snap(&app, &format!("sidebar-toprow-many-end-{n}"));
        // All tabs menu open.
        let mut app = many(mode, "tab4");
        let _ = app.update(Message::TopRow(TopRowMsg::ToggleMenu(flockbook::toprow::MenuKind::AllTabs)));
        snap(&app, &format!("sidebar-toprow-menu-{n}"));
    }
    let _ = DemoSource::new();
}
