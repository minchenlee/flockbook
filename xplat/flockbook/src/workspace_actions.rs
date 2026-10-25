//! Explicit-target topology actions and in-window editors. Socket I/O never runs on the UI thread.
use crate::{App, Message};
use fb_ui::theme::*;
pub use herdr_core::client::WorkspaceAction;
use iced::widget::{button, column, container, mouse_area, row, stack, text_input, Space};
use iced::{Element, Length, Task};
use std::collections::HashSet;
use std::sync::Arc;

pub use crate::settings::PlusAction;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Workspace(String),
    Tab(String),
    Pane(String),
}
#[derive(Debug, Clone)]
pub enum Editor {
    CreateWorkspace,
    Rename(Target),
}
#[derive(Debug, Clone)]
pub enum WorkspaceMsg {
    Menu(Target),
    Dismiss,
    Edit(Editor),
    Name(String),
    Cwd(String),
    Save,
    Run(WorkspaceAction),
    Confirm(WorkspaceAction),
    Finished(Result<serde_json::Value, String>),
    Zoom(String),
    OpenTerminal(String),
    NewTab,
    Plus,
}
pub trait WorkspaceWriter: Send + Sync {
    fn run(&self, action: &WorkspaceAction) -> Result<serde_json::Value, String>;
}
#[derive(Default)]
pub struct WorkspaceActionsState {
    pub writer: Option<Arc<dyn WorkspaceWriter>>,
    pub menu: Option<Target>,
    pub editor: Option<Editor>,
    pub name: String,
    pub cwd: String,
    pub confirmation: Option<WorkspaceAction>,
    pub error: Option<String>,
    pub busy: bool,
    pub right_click: HashSet<String>,
    /// Swift uses the last requested direction for the whole reading layout.
    pub split_down: bool,
}
#[cfg(unix)]
struct LiveWriter {
    client: herdr_core::client::HerdrClient<herdr_core::transport::UnixConnector>,
    session: herdr_core::HerdrSession,
}
#[cfg(unix)]
impl WorkspaceWriter for LiveWriter {
    fn run(&self, a: &WorkspaceAction) -> Result<serde_json::Value, String> {
        self.client
            .workspace_action(&self.session, a)
            .map_err(|e| e.to_string())
    }
}
#[cfg(unix)]
pub fn install_live(app: &mut App, session: &herdr_core::HerdrSession) {
    if session.allows_workspace_writes() {
        app.state.workspace_actions.writer = Some(Arc::new(LiveWriter {
            client: herdr_core::client::HerdrClient::new(session),
            session: session.clone(),
        }));
    }
}
pub fn writable(app: &App) -> bool {
    app.state.workspace_actions.writer.is_some()
        && crate::access::writes_live(app)
        && app.state.settings.prefs.allow_input != Some(false)
        && !app.state.settings.read_only_launch
}
pub fn modal(app: &App) -> bool {
    let s = &app.state.workspace_actions;
    s.menu.is_some()
        || s.editor.is_some()
        || s.confirmation.is_some()
        || s.busy
        || s.error.is_some()
}
fn msg(m: WorkspaceMsg) -> Message {
    Message::Workspace(m)
}
fn action(a: WorkspaceAction) -> Message {
    msg(WorkspaceMsg::Run(a))
}
fn find_tab<'a>(
    app: &'a App,
    id: &str,
) -> Option<(&'a herdr_core::Workspace, &'a herdr_core::Tab)> {
    app.state
        .workspaces
        .iter()
        .find_map(|w| w.tabs.iter().find(|t| t.id == id).map(|t| (w, t)))
}
fn exists(app: &App, a: &WorkspaceAction) -> bool {
    match a {
        WorkspaceAction::CreateWorkspace { .. } => true,
        WorkspaceAction::RenameWorkspace { id, .. } | WorkspaceAction::CloseWorkspace { id } => {
            app.state.workspace(id).is_some()
        }
        WorkspaceAction::CreateTab { workspace, .. } => app.state.workspace(workspace).is_some(),
        WorkspaceAction::RenameTab { id, .. } | WorkspaceAction::CloseTab { id } => {
            find_tab(app, id).is_some()
        }
        WorkspaceAction::MoveTab { id, workspace, .. } => {
            find_tab(app, id).is_some()
                && workspace
                    .as_deref()
                    .is_none_or(|w| app.state.workspace(w).is_some())
        }
        WorkspaceAction::Split { pane: id, .. }
        | WorkspaceAction::RenamePane { id, .. }
        | WorkspaceAction::ClosePane { id }
        | WorkspaceAction::RightClick { pane: id, .. } => app
            .state
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .flat_map(|t| &t.panes)
            .any(|p| &p.id == id),
    }
}
fn new_tab(app: &mut App) -> Task<Message> {
    let Some(w) = app.state.selected_workspace() else {
        return Task::none();
    };
    let a = WorkspaceAction::CreateTab {
        workspace: w.id.clone(),
        cwd: w
            .tabs
            .first()
            .and_then(|t| t.panes.first())
            .map(|p| p.cwd.clone())
            .unwrap_or_default(),
    };
    update(app, WorkspaceMsg::Run(a))
}
pub fn update(app: &mut App, m: WorkspaceMsg) -> Task<Message> {
    match m {
        WorkspaceMsg::Menu(t) => {
            app.state.workspace_actions.menu = Some(t);
            app.state.workspace_actions.error = None;
        }
        WorkspaceMsg::Dismiss => {
            let s = &mut app.state.workspace_actions;
            s.menu = None;
            s.editor = None;
            s.confirmation = None;
            s.error = None;
        }
        WorkspaceMsg::Edit(e) => {
            if !writable(app) {
                return Task::none();
            }
            let name = match &e {
                Editor::CreateWorkspace => String::new(),
                Editor::Rename(Target::Workspace(id)) => app
                    .state
                    .workspace(id)
                    .map(|w| w.label.clone())
                    .unwrap_or_default(),
                Editor::Rename(Target::Tab(id)) => find_tab(app, id)
                    .map(|(_, t)| t.label.clone())
                    .unwrap_or_default(),
                Editor::Rename(Target::Pane(id)) => app
                    .state
                    .workspaces
                    .iter()
                    .flat_map(|w| &w.tabs)
                    .flat_map(|t| &t.panes)
                    .find(|p| &p.id == id)
                    .and_then(|p| p.label.clone())
                    .unwrap_or_default(),
            };
            let s = &mut app.state.workspace_actions;
            s.editor = Some(e);
            s.menu = None;
            s.name = name;
            s.cwd.clear();
        }
        WorkspaceMsg::Name(n) => app.state.workspace_actions.name = n,
        WorkspaceMsg::Cwd(n) => app.state.workspace_actions.cwd = n,
        WorkspaceMsg::Save => {
            let s = &app.state.workspace_actions;
            let label = s.name.trim().to_string();
            let a = match &s.editor {
                Some(Editor::CreateWorkspace) => WorkspaceAction::CreateWorkspace {
                    label,
                    cwd: s.cwd.clone(),
                },
                Some(Editor::Rename(Target::Workspace(id))) if !label.is_empty() => {
                    WorkspaceAction::RenameWorkspace {
                        id: id.clone(),
                        label,
                    }
                }
                Some(Editor::Rename(Target::Tab(id))) if !label.is_empty() => {
                    WorkspaceAction::RenameTab {
                        id: id.clone(),
                        label,
                    }
                }
                Some(Editor::Rename(Target::Pane(id))) => WorkspaceAction::RenamePane {
                    id: id.clone(),
                    label,
                },
                _ => return Task::none(),
            };
            return update(app, WorkspaceMsg::Run(a));
        }
        WorkspaceMsg::Confirm(a) => {
            if !writable(app) {
                return Task::none();
            }
            app.state.workspace_actions.confirmation = Some(a);
            app.state.workspace_actions.menu = None;
        }
        WorkspaceMsg::Run(a) => {
            if !writable(app) || app.state.workspace_actions.busy || !exists(app, &a) {
                return Task::none();
            }
            if let WorkspaceAction::MoveTab { id, .. } = &a {
                let sending = find_tab(app, id).is_some_and(|(_, t)| {
                    t.panes
                        .iter()
                        .any(|p| app.state.composer.pending.contains_key(&p.id))
                });
                if sending || app.state.composer.pulling {
                    app.state.workspace_actions.error = Some(
                        "Wait for the current send or queue pull to finish before moving this tab."
                            .into(),
                    );
                    return Task::none();
                }
            }
            // Closing an occupied container always needs the explicit confirmation path.
            let destructive = matches!(
                a,
                WorkspaceAction::CloseWorkspace { .. }
                    | WorkspaceAction::CloseTab { .. }
                    | WorkspaceAction::ClosePane { .. }
            );
            if destructive && app.state.workspace_actions.confirmation.as_ref() != Some(&a) {
                return update(app, WorkspaceMsg::Confirm(a));
            }
            let Some(writer) = app.state.workspace_actions.writer.clone() else {
                return Task::none();
            };
            let s = &mut app.state.workspace_actions;
            s.busy = true;
            s.error = None;
            s.menu = None;
            if let WorkspaceAction::Split { down, .. } = &a {
                s.split_down = *down;
            }
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let result = writer.run(&a).map(|mut v| {
                    if let WorkspaceAction::RightClick { pane, routed } = &a {
                        v["routing"] = serde_json::json!({"pane": pane, "routed": routed});
                    }
                    v
                });
                let _ = tx.send(result);
            });
            return Task::perform(
                async move {
                    rx.await
                        .unwrap_or_else(|_| Err("workspace worker stopped".into()))
                },
                |r| msg(WorkspaceMsg::Finished(r)),
            );
        }
        WorkspaceMsg::Finished(r) => {
            let s = &mut app.state.workspace_actions;
            s.busy = false;
            match r {
                Err(e) => s.error = Some(e),
                Ok(v) => {
                    s.editor = None;
                    s.confirmation = None;
                    s.error = v["warning"].as_str().map(str::to_owned);
                    if let (Some(p), Some(on)) = (
                        v["routing"]["pane"].as_str(),
                        v["routing"]["routed"].as_bool(),
                    ) {
                        if on {
                            s.right_click.insert(p.into());
                        } else {
                            s.right_click.remove(p);
                        }
                    }
                    let workspace = v["result"]["workspace"]["workspace_id"].as_str();
                    if let Some(w) = workspace {
                        app.state.pending_workspace = Some(w.into());
                    }
                    let tab = v["result"]["tab"]["tab_id"].as_str();
                    if let Some(t) = tab {
                        app.state.pending_tab = Some(t.into());
                    }
                    if let Some(remaps) = v["moved_panes"].as_array() {
                        for remap in remaps {
                            if let (Some(old), Some(new)) =
                                (remap["from"].as_str(), remap["to"].as_str())
                            {
                                if app.state.composer.pane.as_deref() == Some(old) {
                                    app.state.composer.pane = Some(new.into());
                                }
                                if let Some(draft) = app.state.composer.drafts.remove(old) {
                                    app.state.composer.drafts.insert(new.into(), draft);
                                }
                                if let Some(images) = app.state.composer.image_drafts.remove(old) {
                                    app.state.composer.image_drafts.insert(new.into(), images);
                                }
                                if let Some(failed) = app.state.composer.failed.remove(old) {
                                    app.state.composer.failed.insert(new.into(), failed);
                                }
                                app.state.composer.queues.remove(old);
                                app.state.composer.queue_logs.remove(old);
                                if app.state.workspace_actions.right_click.remove(old) {
                                    app.state.workspace_actions.right_click.insert(new.into());
                                }
                                if app.state.selected_pane.as_deref() == Some(old) {
                                    app.state.selected_pane = Some(new.into());
                                }
                            }
                        }
                    }
                    app.refresh();
                }
            }
        }
        WorkspaceMsg::Zoom(p) => {
            if app
                .state
                .selected_tab()
                .is_some_and(|t| t.panes.iter().any(|x| x.id == p))
            {
                let tab = app.state.selected_tab.clone().unwrap();
                if app.state.zoomed.get(&tab) == Some(&p) {
                    app.state.zoomed.remove(&tab);
                } else {
                    app.state.zoomed.insert(tab, p.clone());
                    app.state.selected_pane = Some(p);
                }
                app.state.workspace_actions.menu = None;
                app.refresh();
            }
        }
        WorkspaceMsg::NewTab => return new_tab(app),
        WorkspaceMsg::OpenTerminal(pane) => {
            if !app
                .state
                .selected_tab()
                .is_some_and(|t| t.panes.iter().any(|p| p.id == pane))
            {
                return Task::none();
            }
            app.state.selected_pane = Some(pane);
            app.refresh();
            crate::terminal::toggle(app);
        }
        WorkspaceMsg::Plus => match app
            .state
            .settings
            .prefs
            .plus_action
            .unwrap_or_default()
            .resolved(app.state.selected_pane().is_some())
        {
            PlusAction::NewTab => return new_tab(app),
            p => {
                if let Some(pane) = app.state.selected_pane() {
                    return update(
                        app,
                        WorkspaceMsg::Run(WorkspaceAction::Split {
                            pane: pane.id.clone(),
                            down: p == PlusAction::SplitDown,
                        }),
                    );
                }
            }
        },
    }
    Task::none()
}

/// Context menus, rename/create form and close confirmation all stay inside the application window.
pub fn layer(app: &App) -> Element<'_, Message> {
    let s = &app.state.workspace_actions;
    if s.menu.is_none()
        && s.editor.is_none()
        && s.confirmation.is_none()
        && s.error.is_none()
        && !s.busy
    {
        return Space::new().into();
    }
    let t = app.tokens();
    let mut col = column![].spacing(8);
    let enabled = writable(app) && !s.busy;
    let entry = |label: String, m: Message, allowed: bool| {
        let b = button(BODY.text(label)).width(Length::Fill);
        if allowed {
            b.on_press(m)
        } else {
            b
        }
    };
    if let Some(target) = &s.menu {
        col = col.push(entry(
            "Rename…".into(),
            msg(WorkspaceMsg::Edit(Editor::Rename(target.clone()))),
            enabled,
        ));
        match target {
            Target::Workspace(id) => {
                col = col.push(entry(
                    "New tab in workspace".into(),
                    action(WorkspaceAction::CreateTab {
                        workspace: id.clone(),
                        cwd: String::new(),
                    }),
                    enabled,
                ));
                col = col.push(entry(
                    "Close workspace…".into(),
                    action(WorkspaceAction::CloseWorkspace { id: id.clone() }),
                    enabled,
                ));
            }
            Target::Tab(id) => {
                if let Some((ws, tab)) = find_tab(app, id) {
                    let index = ws.tabs.iter().position(|t| t.id == *id).unwrap_or(0);
                    for (label, to, allowed) in [
                        ("Move left", index.saturating_sub(1), index > 0),
                        ("Move right", index + 1, index + 1 < ws.tabs.len()),
                    ] {
                        col = col.push(entry(
                            label.into(),
                            action(WorkspaceAction::MoveTab {
                                id: id.clone(),
                                workspace: None,
                                index: to,
                            }),
                            enabled && allowed,
                        ));
                    }
                    for other in &app.state.workspaces {
                        if other.id != ws.id {
                            col = col.push(entry(
                                format!("Move to {}", other.label),
                                action(WorkspaceAction::MoveTab {
                                    id: id.clone(),
                                    workspace: Some(other.id.clone()),
                                    index: other.tabs.len(),
                                }),
                                enabled,
                            ));
                        }
                    }
                    if let Some(p) = tab.panes.first() {
                        for (label, down) in [("Split right", false), ("Split down", true)] {
                            col = col.push(entry(
                                label.into(),
                                action(WorkspaceAction::Split {
                                    pane: p.id.clone(),
                                    down,
                                }),
                                enabled,
                            ));
                        }
                    }
                    col = col.push(entry(
                        "Close tab…".into(),
                        action(WorkspaceAction::CloseTab { id: id.clone() }),
                        enabled,
                    ));
                }
            }
            Target::Pane(id) => {
                for (label, down) in [("Split right", false), ("Split down", true)] {
                    col = col.push(entry(
                        label.into(),
                        action(WorkspaceAction::Split {
                            pane: id.clone(),
                            down,
                        }),
                        enabled,
                    ));
                }
                let zoomed = app
                    .state
                    .selected_tab
                    .as_ref()
                    .and_then(|t| app.state.zoomed.get(t))
                    == Some(id);
                col = col.push(entry(
                    if zoomed { "Restore panes" } else { "Zoom" }.into(),
                    msg(WorkspaceMsg::Zoom(id.clone())),
                    true,
                ));
                let routed = s.right_click.contains(id);
                col = col.push(entry(
                    if routed {
                        "✓ Send right-clicks to pane"
                    } else {
                        "Send right-clicks to pane"
                    }
                    .into(),
                    action(WorkspaceAction::RightClick {
                        pane: id.clone(),
                        routed: !routed,
                    }),
                    enabled,
                ));
                col = col.push(entry(
                    "Close pane…".into(),
                    action(WorkspaceAction::ClosePane { id: id.clone() }),
                    enabled,
                ));
            }
        }
    }
    if let Some(e) = &s.editor {
        col = col.push(TITLE_3.text(if matches!(e, Editor::CreateWorkspace) {
            "New workspace"
        } else {
            "Rename"
        }));
        col = col.push(
            text_input("Name", &s.name)
                .on_input(|n| msg(WorkspaceMsg::Name(n)))
                .on_submit(msg(WorkspaceMsg::Save)),
        );
        if matches!(e, Editor::CreateWorkspace) {
            col = col.push(
                text_input("Start folder (optional)", &s.cwd)
                    .on_input(|n| msg(WorkspaceMsg::Cwd(n))),
            );
        }
        col = col.push(entry("Save".into(), msg(WorkspaceMsg::Save), enabled));
    }
    if let Some(a) = &s.confirmation {
        col = col.push(BODY.text("Close this item and terminate its panes and running processes?"));
        col = col.push(entry(
            "Close and terminate".into(),
            action(a.clone()),
            enabled,
        ));
    }
    if s.busy {
        col = col.push(BODY.text("Working…"));
    }
    if let Some(e) = &s.error {
        col = col.push(BODY.text(e.clone()).color(t.blocked));
    }
    col = col.push(button(BODY.text("Cancel")).on_press(msg(WorkspaceMsg::Dismiss)));
    let card = container(col)
        .padding(16)
        .width(360)
        .style(crate::style::float_e2(t, R_MENU));
    stack![
        mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_press(msg(WorkspaceMsg::Dismiss)),
        container(card).center(Length::Fill)
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Recorder(Mutex<Vec<WorkspaceAction>>);
    impl WorkspaceWriter for Recorder {
        fn run(&self, a: &WorkspaceAction) -> Result<serde_json::Value, String> {
            self.0.lock().unwrap().push(a.clone());
            Ok(serde_json::json!({}))
        }
    }
    struct Sender;
    impl crate::composer::MessageSender for Sender {
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    fn demo() -> App {
        let mut app = App::demo(fb_ui::Mode::Light, crate::state::Screen::Conversation);
        app.state.composer.sender = Some(Arc::new(Sender));
        app
    }
    #[test]
    fn no_backend_and_input_off_each_prevent_topology_writes() {
        let mut app = demo();
        assert!(!writable(&app));
        app.state.workspace_actions.writer = Some(Arc::new(Recorder::default()));
        assert!(writable(&app));
        crate::access::make_read_only(&mut app);
        assert!(!writable(&app));
        let _ = update(
            &mut app,
            WorkspaceMsg::Run(WorkspaceAction::CreateWorkspace {
                label: "Synthetic".into(),
                cwd: String::new(),
            }),
        );
        assert!(!app.state.workspace_actions.busy);
    }
    #[test]
    fn closing_requires_confirmation_before_starting_a_worker() {
        let mut app = demo();
        app.state.workspace_actions.writer = Some(Arc::new(Recorder::default()));
        let id = app.state.selected_tab.clone().unwrap();
        let a = WorkspaceAction::CloseTab { id };
        let _ = update(&mut app, WorkspaceMsg::Run(a.clone()));
        assert_eq!(app.state.workspace_actions.confirmation, Some(a));
        assert!(!app.state.workspace_actions.busy);
    }
    #[test]
    fn local_zoom_restores_without_any_write_backend() {
        let mut app = demo();
        let p = app.state.selected_pane().unwrap().id.clone();
        let t = app.state.selected_tab.clone().unwrap();
        let _ = update(&mut app, WorkspaceMsg::Zoom(p.clone()));
        assert_eq!(app.state.zoomed.get(&t), Some(&p));
        let _ = update(&mut app, WorkspaceMsg::Zoom(p));
        assert!(!app.state.zoomed.contains_key(&t));
    }
    #[test]
    fn plus_with_no_pane_resolves_to_new_tab() {
        assert_eq!(PlusAction::SplitRight.resolved(false), PlusAction::NewTab);
        assert_eq!(PlusAction::SplitDown.resolved(true), PlusAction::SplitDown);
    }
}
