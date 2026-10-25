//! Split readers: independent documents, selections, scroll anchors and paging tickets per pane.
use crate::workspace_actions::{Target, WorkspaceMsg};
use crate::{state::State, App, Message};
use fb_ui::selectable::flow::Flow;
use fb_ui::theme::*;
use iced::widget::{button, column, container, mouse_area, row, Space};
use iced::{Element, Length, Task};

/// Move (never clone) the active reader into its pane's slot when selection changes.
pub fn switch_reader(s: &mut State, pane: Option<&str>) {
    if s.conversation.pane.as_deref() == pane {
        return;
    }
    let old = std::mem::take(&mut s.conversation);
    if let Some(id) = old.pane.clone() {
        if s.workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .flat_map(|t| &t.panes)
            .any(|p| p.id == id)
        {
            s.pane_readers.insert(id, old);
        }
    }
    if let Some(p) = pane {
        s.conversation = s.pane_readers.remove(p).unwrap_or_default();
    }
}

/// Prepares all readers in the selected tab without touching composer drafts or card targets.
pub fn refresh_other_readers(app: &mut App) {
    let active = app.state.selected_pane().map(|p| p.id.clone());
    let selected = app.state.selected_pane.clone();
    let panes = app
        .state
        .selected_tab()
        .map(|t| t.panes.clone())
        .unwrap_or_default();
    for p in panes {
        if Some(&p.id) == active.as_ref() || p.agent.is_none() {
            continue;
        }
        let changed = app.state.reload_pane(app.source.as_ref(), &p);
        if !changed && app.state.pane_readers.contains_key(&p.id) {
            continue;
        }
        switch_reader(&mut app.state, Some(&p.id));
        app.state.selected_pane = Some(p.id);
        crate::conversation::sync_markdown(app);
    }
    switch_reader(&mut app.state, active.as_deref());
    app.state.selected_pane = selected;
    release_hidden_pictures(&mut app.state);
}

/// Hidden tabs and zoomed-out panes must not retain image handles outside the bounded image cache.
pub fn release_hidden_pictures(s: &mut State) {
    let visible: std::collections::HashSet<String> =
        if s.screen == crate::state::Screen::Conversation {
            s.selected_tab()
                .map(|t| {
                    let zoom = s.zoomed.get(&t.id);
                    t.panes
                        .iter()
                        .filter(|p| zoom.is_none_or(|id| id == &p.id))
                        .map(|p| p.id.clone())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Default::default()
        };
    for (id, cs) in &mut s.pane_readers {
        if !visible.contains(id) {
            cs.pictures.clear_visible();
        }
    }
    if s.conversation
        .pane
        .as_ref()
        .is_none_or(|id| !visible.contains(id))
    {
        s.conversation.pictures.clear_visible();
    }
}

/// Widget/paging results carry their pane id; a background result cannot overwrite another reader.
pub fn update_reader(
    app: &mut App,
    pane: String,
    message: crate::conversation::ConversationMsg,
) -> Task<Message> {
    let location = app.state.workspaces.iter().find_map(|w| {
        w.tabs
            .iter()
            .find(|t| t.panes.iter().any(|p| p.id == pane))
            .map(|t| (w.id.clone(), t.id.clone()))
    });
    let Some((ws, tab)) = location else {
        return Task::none();
    };
    // Clicking or scrolling a split selects it; asynchronous paging completion keeps focus where it was.
    let background = matches!(
        message,
        crate::conversation::ConversationMsg::EarlierLoaded { .. }
            | crate::conversation::ConversationMsg::PictureLoaded { .. }
            | crate::conversation::ConversationMsg::AutoScroll
            | crate::conversation::ConversationMsg::Flow(
                fb_ui::selectable::flow::FlowEvent::PicturesNear(_)
                    | fb_ui::selectable::flow::FlowEvent::Viewport(_)
                    | fb_ui::selectable::flow::FlowEvent::Settle
                    | fb_ui::selectable::flow::FlowEvent::Blur
                    | fb_ui::selectable::flow::FlowEvent::Release
            )
    );
    let selected = app.state.selected_pane.clone();
    let selected_ws = app.state.selected_workspace.clone();
    let selected_tab = app.state.selected_tab.clone();
    let active = app.state.selected_pane().map(|p| p.id.clone());
    switch_reader(&mut app.state, Some(&pane));
    app.state.selected_pane = Some(pane);
    app.state.selected_workspace = Some(ws);
    app.state.selected_tab = Some(tab);
    if !background {
        app.state
            .composer
            .switch_pane(app.state.selected_pane.as_deref());
    }
    let task =
        if let crate::conversation::ConversationMsg::EarlierLoaded { ticket, more, .. } = message {
            let cs = &mut app.state.conversation;
            if cs.earlier_seq == ticket && cs.loading_earlier {
                cs.loading_earlier = false;
                if let Some(m) = more {
                    cs.more_earlier = m;
                }
                if let Some(p) = app.state.selected_pane().cloned() {
                    app.state.invalidate(&p.id);
                    app.state.reload_pane(app.source.as_ref(), &p);
                    crate::conversation::sync_markdown(app);
                }
            }
            Task::none()
        } else {
            crate::conversation::update(app, message)
        };
    if background {
        switch_reader(&mut app.state, active.as_deref());
        app.state.selected_pane = selected;
        app.state.selected_workspace = selected_ws;
        app.state.selected_tab = selected_tab;
    } else {
        crate::cards::on_refresh(app);
    }
    release_hidden_pictures(&mut app.state);
    task
}

fn reader<'a>(app: &'a App, pane: &'a herdr_core::Pane) -> Element<'a, Message> {
    let active = app.state.selected_pane().is_some_and(|p| p.id == pane.id);
    let t = app.tokens();
    let label = pane
        .label
        .clone()
        .or_else(|| pane.agent.as_ref().map(|a| a.name.clone()))
        .unwrap_or_else(|| "Shell".into());
    let header = mouse_area(row![
        button(BODY.text(label))
            .on_press(Message::SelectPane(pane.id.clone()))
            .width(Length::Fill),
        button(BODY.text("⋯")).on_press(Message::Workspace(WorkspaceMsg::Menu(Target::Pane(
            pane.id.clone()
        )))),
    ])
    .on_right_press(Message::Workspace(WorkspaceMsg::Menu(Target::Pane(
        pane.id.clone(),
    ))));
    if pane.agent.is_none() {
        let compact = column![
            BODY.text("Shell").color(t.label),
            CAPTION.text(pane.cwd.clone()).color(t.label_2),
            button(BODY.text("Show terminal")).on_press(Message::Workspace(
                WorkspaceMsg::OpenTerminal(pane.id.clone())
            ))
        ]
        .spacing(8);
        return container(column![
            header,
            container(compact).padding(16),
            Space::new().height(Length::Fill)
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::style::card_e1(t, R_ROW))
        .into();
    }
    let content = if active {
        let id = pane.id.clone();
        crate::conversation::view(app).map(move |m| match m {
            Message::Conversation(message) => Message::PaneConversation {
                pane: id.clone(),
                message,
            },
            m => m,
        })
    } else if let Some(cs) = app.state.pane_readers.get(&pane.id) {
        let id = pane.id.clone();
        Element::from(
            Flow::new(&cs.doc, t, move |e| Message::PaneConversation {
                pane: id.clone(),
                message: crate::conversation::ConversationMsg::Flow(e),
            })
            .col_max(app.state.settings.column_width())
            .offset(cs.offset, cs.pinned)
            .selection(cs.sel.sel)
            .focused(false)
            .dragging(cs.sel.dragging())
            .viewport(cs.viewport)
            .measure_into(cs.sink.clone())
            .picture_scope(&cs.session_key)
            .picture_epoch(cs.pictures.load_seq)
            .pictures(&cs.pictures.handles),
        )
    } else {
        container(BODY.text("Loading conversation…").color(t.label_2))
            .center(Length::Fill)
            .into()
    };
    let mut col = column![header, container(content).height(Length::Fill)];
    if active {
        col = col.push(crate::cards::bottom(app));
    }
    container(col)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let Some(tab) = app.state.selected_tab() else {
        return crate::conversation::view(app);
    };
    let zoom = app.state.zoomed.get(&tab.id);
    let panes: Vec<_> = tab
        .panes
        .iter()
        .filter(|p| zoom.is_none_or(|id| id == &p.id))
        .collect();
    if panes.len() == 1 {
        return reader(app, panes[0]);
    }
    // Like Swift, the last requested split direction controls the reading layout.
    if app.state.workspace_actions.split_down {
        let mut col = column![]
            .spacing(1)
            .width(Length::Fill)
            .height(Length::Fill);
        for p in panes {
            let height = if p.agent.is_none() {
                Length::Fixed(150.0)
            } else {
                Length::Fill
            };
            col = col.push(container(reader(app, p)).height(height));
        }
        col.into()
    } else {
        let mut r = row![].spacing(1).width(Length::Fill).height(Length::Fill);
        for p in panes {
            r = r.push(reader(app, p));
        }
        r.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    struct SyntheticSource(Arc<Mutex<Vec<herdr_core::Workspace>>>);
    impl crate::source::DataSource for SyntheticSource {
        fn label(&self) -> String {
            "synthetic".into()
        }
        fn workspaces(&self) -> Vec<herdr_core::Workspace> {
            self.0.lock().unwrap().clone()
        }
        fn connected(&self) -> bool {
            true
        }
        fn revision(&self, _: &herdr_core::Pane) -> Option<u64> {
            Some(1)
        }
        fn conversation(&self, p: &herdr_core::Pane) -> Vec<herdr_core::ConversationItem> {
            vec![herdr_core::ConversationItem {
                id: p.id.clone(),
                kind: herdr_core::ItemKind::AssistantText(
                    p.agent
                        .as_ref()
                        .and_then(|a| a.session_id.clone())
                        .unwrap_or_default(),
                ),
                timestamp: None,
            }]
        }
    }
    fn synthetic() -> (App, Arc<Mutex<Vec<herdr_core::Workspace>>>) {
        use herdr_core::{Agent, AgentStatus, Pane, Tab, Workspace};
        let panes = (1..=2)
            .map(|i| Pane {
                id: format!("w1:p{i}"),
                terminal_id: format!("term{i}"),
                cwd: String::new(),
                label: None,
                agent: Some(Agent {
                    name: format!("synthetic{i}"),
                    kind: "claude".into(),
                    status: AgentStatus::Idle,
                    session_id: Some(format!("session{i}")),
                    title: String::new(),
                }),
            })
            .collect();
        let tree = Arc::new(Mutex::new(vec![Workspace {
            id: "w1".into(),
            label: "Synthetic".into(),
            tabs: vec![Tab {
                id: "w1:t1".into(),
                label: "Synthetic".into(),
                number: 1,
                panes,
                status: AgentStatus::Idle,
            }],
        }]));
        let app = App::new(
            Box::new(SyntheticSource(tree.clone())),
            fb_ui::Mode::Light,
            crate::state::Screen::Conversation,
        );
        (app, tree)
    }
    #[test]
    fn passive_split_layout_events_do_not_change_selection_or_draft_target() {
        let (mut app, _) = synthetic();
        let before = app.state.selected_pane().unwrap().id.clone();
        let draft_target = app.state.composer.pane.clone();
        let _ = update_reader(
            &mut app,
            "w1:p2".into(),
            crate::conversation::ConversationMsg::Flow(
                fb_ui::selectable::flow::FlowEvent::Viewport(iced::Size::new(320.0, 200.0)),
            ),
        );
        assert_eq!(app.state.selected_pane().unwrap().id, before);
        assert_eq!(app.state.composer.pane, draft_target);
        assert_eq!(
            app.state.pane_readers["w1:p2"].viewport,
            iced::Size::new(320.0, 200.0)
        );
    }
    #[test]
    fn same_revision_with_new_session_replaces_the_old_reader() {
        let (mut app, tree) = synthetic();
        app.state.conversation.offset = 88.0;
        app.state.conversation.pinned = false;
        tree.lock().unwrap()[0].tabs[0].panes[0]
            .agent
            .as_mut()
            .unwrap()
            .session_id = Some("replacement".into());
        app.refresh();
        assert!(app.state.conversation.pinned);
        assert!(
            matches!(&app.state.conversation_items()[0].kind, herdr_core::ItemKind::AssistantText(t) if t == "replacement")
        );
    }
    #[test]
    fn background_paging_completion_updates_only_its_reader() {
        let (mut app, _) = synthetic();
        let cs = app.state.pane_readers.get_mut("w1:p2").unwrap();
        cs.loading_earlier = true;
        cs.earlier_seq = 9;
        let _ = update_reader(
            &mut app,
            "w1:p2".into(),
            crate::conversation::ConversationMsg::EarlierLoaded {
                pane: "w1:p2".into(),
                ticket: 9,
                more: Some(false),
            },
        );
        assert!(!app.state.pane_readers["w1:p2"].loading_earlier);
        assert_eq!(app.state.selected_pane().unwrap().id, "w1:p1");
    }
    #[test]
    fn hidden_readers_release_visible_picture_state() {
        let (mut app, _) = synthetic();
        app.state
            .pane_readers
            .get_mut("w1:p2")
            .unwrap()
            .pictures
            .near
            .insert(7);
        app.state.zoomed.insert("w1:t1".into(), "w1:p1".into());
        release_hidden_pictures(&mut app.state);
        assert!(app.state.pane_readers["w1:p2"].pictures.near.is_empty());
    }
    #[test]
    fn switching_restores_reader_anchor_and_selection_state() {
        let mut app = App::demo(fb_ui::Mode::Light, crate::state::Screen::Conversation);
        let old = app.state.conversation.pane.clone().unwrap();
        app.state.conversation.offset = 123.0;
        app.state.conversation.pinned = false;
        switch_reader(&mut app.state, Some("synthetic-pane"));
        assert!(app.state.pane_readers.contains_key(&old));
        switch_reader(&mut app.state, Some(&old));
        assert_eq!(app.state.conversation.offset, 123.0);
        assert!(!app.state.conversation.pinned);
    }
}
