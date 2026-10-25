//! Search screen: tab and message matching (incl. CJK), keyboard navigation, stale results, snapshots.
//! Headless (iced_test Simulator, tiny-skia); no window. Nothing here sends anything to a pane.

use fb_ui::Mode;
use flockbook::search::{self, on_key, Group, SearchMsg};
use flockbook::snapshot::{distinct_colors, render_app, write_png};
use flockbook::state::Screen;
use flockbook::{App, Message};
use herdr_core::{ConversationItem, ItemKind};
use iced::keyboard::{key::Named, Key, Modifiers};
use iced::{Settings, Size};
use iced_test::Simulator;

/// Demo app on the Orchard tab with every pane's conversation loaded and the Search screen open.
fn app(mode: Mode) -> App {
    let mut a = App::demo(mode, Screen::Search);
    let (ws, tab) = {
        let w = a.state.workspaces.iter().find(|w| w.label == "Orchard").unwrap();
        (w.id.clone(), w.tabs.iter().find(|t| t.label == "cart drawer").unwrap().id.clone())
    };
    a.state.select_tab(&ws, &tab);
    a.refresh();
    search::preload_all(&mut a);
    let _ = a.update(Message::Search(SearchMsg::Open));
    a
}

fn input(a: &mut App, q: &str) {
    let _ = a.update(Message::Search(SearchMsg::Input(q.to_string())));
}

fn sim(app: &App, w: f32, h: f32) -> Simulator<'_, Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    Simulator::with_size(settings, Size::new(w, h), app.view())
}

fn tab_labels(a: &App) -> Vec<String> {
    a.state.search.tabs.iter().map(|h| h.tab_label.clone()).collect()
}

#[test]
fn empty_query_lists_every_tab_and_no_messages() {
    let a = app(Mode::Light);
    assert_eq!(a.state.search.tabs.len(), 16);
    assert!(a.state.search.messages.is_empty());
}

#[test]
fn query_matches_tab_workspace_and_agent_names_case_insensitively() {
    let mut a = app(Mode::Light);
    input(&mut a, "RANKING");
    assert_eq!(tab_labels(&a), vec!["ranking"]);
    input(&mut a, "quarry");
    assert_eq!(tab_labels(&a), vec!["ranking", "bench", "notes"], "workspace name matches all its tabs");
    input(&mut a, "harbor-ci");
    assert_eq!(tab_labels(&a), vec!["ci"], "agent name");
    assert!(a.state.search.tabs[0].pane.is_some(), "the matching agent's pane is remembered");
    input(&mut a, "zzzz");
    assert!(a.state.search.is_empty());
}

#[test]
fn query_matches_message_text_including_cjk() {
    let mut a = app(Mode::Light);
    input(&mut a, "GALA APPLES");
    let m = &a.state.search.messages;
    assert!(!m.is_empty(), "english text, any case");
    assert!(m.iter().all(|h| h.tab_label == "ranking"));
    assert!(m.iter().any(|h| h.parts.hit.to_lowercase() == "gala apples"));

    input(&mut a, "衰減");
    let m = &a.state.search.messages;
    assert_eq!(m.len(), 1, "one CJK message");
    assert_eq!(m[0].item, "a3");
    assert_eq!(m[0].parts.hit, "衰減");
    assert!(m[0].parts.before.contains("長尾查詢") && m[0].parts.after.contains("影響"));

    input(&mut a, "中文");
    assert_eq!(a.state.search.messages[0].item, "u3", "the user's CJK message");
    assert_eq!(a.state.search.messages[0].role, search::Role::You);
}

#[test]
fn results_are_grouped_tabs_first_then_messages() {
    let mut a = app(Mode::Light);
    input(&mut a, "rank");
    let s = &a.state.search;
    assert!(!s.tabs.is_empty() && !s.messages.is_empty());
    assert_eq!(s.group_of(0), Group::Tabs);
    assert_eq!(s.group_of(s.tabs.len()), Group::Messages);
}

#[test]
fn enter_opens_a_tab_result() {
    let mut a = app(Mode::Light);
    input(&mut a, "harbor-ci");
    let _ = a.update(Message::Search(SearchMsg::Activate));
    assert_eq!(a.state.screen, Screen::Conversation);
    assert_eq!(a.state.selected_tab().map(|t| t.label.as_str()), Some("ci"));
    assert_eq!(a.state.selected_pane().and_then(|p| p.agent.as_ref()).map(|x| x.name.as_str()), Some("harbor-ci-dev"));
}

#[test]
fn arrows_move_and_enter_opens_a_message_and_scrolls_to_it() {
    let mut a = app(Mode::Light);
    assert_eq!(a.state.selected_tab().map(|t| t.label.as_str()), Some("cart drawer"));
    input(&mut a, "30-day half-life");
    assert_eq!(a.state.search.tabs.len(), 0);
    let n = a.state.search.messages.len();
    assert!(n >= 1);
    let want = a.state.search.messages.iter().position(|h| h.item == "a1").expect("a1 is a hit");
    for _ in 0..want {
        let _ = a.update(Message::Search(SearchMsg::Move(1)));
    }
    assert_eq!(a.state.search.selected, want, "Down moves to the a1 row");
    let _ = a.update(Message::Search(SearchMsg::Activate));
    assert_eq!(a.state.screen, Screen::Conversation);
    assert_eq!(a.state.selected_tab().map(|t| t.label.as_str()), Some("ranking"));
    let cs = &a.state.conversation;
    assert!(!cs.pinned, "no longer following the bottom");
    assert!(cs.offset > 0.0, "scrolled down to the message: {}", cs.offset);
    let idx = cs.doc.index[&fb_ui::selectable::flow::row_id(&["a1", "0"])];
    let top = cs.doc.window.block_top(idx);
    assert!((cs.offset - (top - 64.0)).abs() < 1.0, "the message row sits near the top");
}

#[test]
fn arrow_keys_move_within_the_results() {
    let mut a = app(Mode::Light);
    input(&mut a, "quarry");
    let n = a.state.search.len();
    for _ in 0..n + 3 {
        let _ = a.update(Message::Search(SearchMsg::Move(1)));
    }
    assert_eq!(a.state.search.selected, n - 1, "clamped at the end");
    let _ = a.update(Message::Search(SearchMsg::Move(-1)));
    assert_eq!(a.state.search.selected, n - 2);
    input(&mut a, "quarry-b");
    assert_eq!(a.state.search.selected, 0, "a new query selects the first row");
}

#[test]
fn shortcuts_map_to_messages() {
    let cmd = if cfg!(target_os = "macos") { Modifiers::LOGO } else { Modifiers::CTRL };
    assert!(matches!(on_key(Key::Character("f".into()), cmd, false), Some(Message::Search(SearchMsg::Open))), "Cmd/Ctrl+F opens from anywhere");
    assert!(on_key(Key::Character("f".into()), Modifiers::empty(), true).is_none(), "plain f is typing");
    assert!(matches!(on_key(Key::Named(Named::ArrowDown), Modifiers::empty(), true), Some(Message::Search(SearchMsg::Move(1)))));
    assert!(matches!(on_key(Key::Named(Named::ArrowUp), Modifiers::empty(), true), Some(Message::Search(SearchMsg::Move(-1)))));
    assert!(on_key(Key::Named(Named::ArrowDown), Modifiers::empty(), false).is_none(), "arrows belong to the other screens");
    assert!(matches!(on_key(Key::Named(Named::Escape), Modifiers::empty(), true), Some(Message::Search(SearchMsg::Escape))));
}

#[test]
fn open_from_any_screen_shows_search() {
    let mut a = App::demo(Mode::Light, Screen::Overview);
    let _ = a.update(Message::Search(SearchMsg::Open));
    assert_eq!(a.state.screen, Screen::Search);
    let _ = a.update(Message::Search(SearchMsg::Escape));
    assert_eq!(a.state.screen, Screen::Conversation, "Esc on an empty field leaves");
}

fn big_item(id: &str, needle: &str) -> ConversationItem {
    let filler = "lorem ipsum dolor sit amet ".repeat(8000);
    ConversationItem { id: id.into(), kind: ItemKind::AssistantText(format!("{filler} {needle} {filler}")), timestamp: Some(1.0) }
}

#[test]
fn large_corpus_searches_off_the_ui_thread_and_stale_results_are_dropped() {
    let mut a = app(Mode::Light);
    let pane = a.state.selected_pane().unwrap().id.clone();
    std::sync::Arc::make_mut(a.state.conversations.get_mut(&pane).unwrap()).push(big_item("big", "zebra-token"));
    assert_eq!(a.state.search.async_runs, 0);
    input(&mut a, "zebra");
    assert_eq!(a.state.search.async_runs, 1, "a worker was started");
    assert!(a.state.search.searching);
    assert!(a.state.search.messages.is_empty(), "nothing inline");
    let first_seq = a.state.search.seq;

    // The user types on: the first query's answer arrives late and is dropped.
    input(&mut a, "zebra-t");
    let second_seq = a.state.search.seq;
    assert!(second_seq > first_seq);
    let late = vec![search_hit(&a, "stale")];
    let _ = a.update(Message::Search(SearchMsg::Results { seq: first_seq, hits: late }));
    assert!(a.state.search.messages.is_empty(), "stale results dropped");
    assert!(a.state.search.searching, "still waiting for the newest query");

    let fresh = vec![search_hit(&a, "fresh")];
    let _ = a.update(Message::Search(SearchMsg::Results { seq: second_seq, hits: fresh }));
    assert_eq!(a.state.search.messages.len(), 1);
    assert_eq!(a.state.search.messages[0].item, "fresh");
    assert!(!a.state.search.searching);

    // Typing again clears the old messages at once.
    input(&mut a, "ranking");
    assert!(a.state.search.messages.iter().all(|h| h.item != "fresh"));
}

#[test]
fn a_big_corpus_is_not_built_on_the_ui_thread() {
    let mut a = app(Mode::Light);
    let pane = a.state.selected_pane().unwrap().id.clone();
    std::sync::Arc::make_mut(a.state.conversations.get_mut(&pane).unwrap()).push(big_item("big", "zebra-token"));
    input(&mut a, "zebra");
    input(&mut a, "zebra-t");
    assert_eq!(a.state.search.async_runs, 2);
    assert_eq!(a.state.search.ui_corpus_builds, 0, "the worker builds it, not the UI thread");
}

#[test]
fn the_worker_builds_the_corpus_once_keeps_it_and_the_stale_guard_still_holds() {
    let mut a = app(Mode::Light);
    let pane = a.state.selected_pane().unwrap().id.clone();
    std::sync::Arc::make_mut(a.state.conversations.get_mut(&pane).unwrap()).push(big_item("big", "zebra-token"));
    input(&mut a, "zebra");
    let seq1 = a.state.search.seq;
    // Run the worker's job the way the task does, without a runtime.
    let needle: Vec<char> = "zebra".chars().collect();
    let SearchMsg::Worked { sig, corpus, raws, .. } = search::job(&a, seq1, needle.clone()).run() else { panic!("worker answer") };
    assert!(corpus.is_some(), "nothing cached yet: the worker built the corpus");
    assert_eq!(raws.len(), 1, "found the big item");
    // A newer query started meanwhile: this answer is stale, but its corpus is kept.
    input(&mut a, "zebra-t");
    let _ = a.update(Message::Search(SearchMsg::Done { seq: seq1, sig, corpus, hits: vec![search_hit(&a, "stale")] }));
    assert!(a.state.search.messages.is_empty(), "stale hits dropped");
    // The next job reuses the kept corpus: the worker does not build again, the UI thread never did.
    let SearchMsg::Worked { corpus: again, .. } = search::job(&a, a.state.search.seq, needle).run() else { panic!() };
    assert!(again.is_none(), "cached corpus reused");
    assert_eq!(a.state.search.ui_corpus_builds, 0);
    // A changed conversation changes the signature: the cache is not used.
    std::sync::Arc::make_mut(a.state.conversations.get_mut(&pane).unwrap()).push(big_item("big2", "other"));
    let SearchMsg::Worked { corpus: rebuilt, .. } = search::job(&a, a.state.search.seq, vec!['z']).run() else { panic!() };
    assert!(rebuilt.is_some());
}

#[test]
fn a_small_corpus_is_built_once_and_rebuilt_only_when_a_conversation_changes() {
    let mut a = app(Mode::Light);
    input(&mut a, "cart");
    input(&mut a, "carts");
    input(&mut a, "drawer");
    assert_eq!(a.state.search.ui_corpus_builds, 1, "same conversations: one build");
    let pane = a.state.selected_pane().unwrap().id.clone();
    std::sync::Arc::make_mut(a.state.conversations.get_mut(&pane).unwrap()).push(ConversationItem { id: "new1".into(), kind: ItemKind::UserText("brand new words".into()), timestamp: Some(2.0) });
    input(&mut a, "brand new");
    assert_eq!(a.state.search.ui_corpus_builds, 2, "a new item changes the signature");
    assert!(a.state.search.messages.iter().any(|h| h.item == "new1"));
}

fn search_hit(a: &App, item: &str) -> search::MsgHit {
    let t = &a.state.search.tabs.first().cloned().unwrap_or_else(|| {
        let ws = &a.state.workspaces[0];
        search::TabHit { workspace: ws.id.clone(), workspace_label: ws.label.clone(), tab: ws.tabs[0].id.clone(), tab_label: ws.tabs[0].label.clone(), pane: None, agent: None, status: ws.tabs[0].status }
    });
    search::MsgHit {
        workspace: t.workspace.clone(),
        workspace_label: t.workspace_label.clone(),
        tab: t.tab.clone(),
        tab_label: t.tab_label.clone(),
        pane: a.state.selected_pane().unwrap().id.clone(),
        agent: None,
        status: t.status,
        item: item.into(),
        role: search::Role::Agent,
        parts: search::Parts { before: "a ".into(), hit: "zebra".into(), after: " b".into() },
    }
}

#[test]
fn the_field_is_wired_to_the_search() {
    let mut a = app(Mode::Light);
    let msgs: Vec<Message> = {
        let mut s = sim(&a, 1280.0, 852.0);
        s.click("Search tabs, agents and messages").expect("field");
        s.typewrite("gala");
        s.into_messages().collect()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Search(SearchMsg::Input(q)) if q == "g")), "typing sends Input");
    for m in msgs {
        let _ = a.update(m);
    }
    assert_eq!(a.state.search.query, "gala");
    assert!(!a.state.search.messages.is_empty());
}

#[test]
fn clicking_a_row_opens_it() {
    let mut a = app(Mode::Light);
    input(&mut a, "bench");
    let msgs: Vec<Message> = {
        let mut s = sim(&a, 1280.0, 852.0);
        // First tab row: below the top row (52) and the list's top inset (7); rows are 32 tall. The field is in the
        // toolbar, as on the Mac. The Mac list has no group headers.
        s.point_at(iced::Point::new(700.0, 52.0 + 7.0 + 16.0));
        s.simulate(iced_test::simulator::click());
        s.into_messages().collect()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Search(SearchMsg::Click(0)))));
    for m in msgs {
        let _ = a.update(m);
    }
    assert_eq!(a.state.selected_tab().map(|t| t.label.as_str()), Some("bench"));
}

#[test]
fn snapshots_light_and_dark() {
    for (mode, name) in [(Mode::Light, "light"), (Mode::Dark, "dark")] {
        for (q, file) in [("rank", "search"), ("", "search-all"), ("衰減", "search-cjk"), ("zzzz", "search-empty")] {
            let mut a = app(mode);
            input(&mut a, q);
            if file == "search" {
                let _ = a.update(Message::Search(SearchMsg::Move(1)));
            }
            let img = render_app(&a, (1280, 852));
            assert!(distinct_colors(&img) > 20, "{file}-{name} is not blank");
            write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{file}-{name}.png"))).unwrap();
        }
    }
}
